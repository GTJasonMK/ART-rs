use chrono::{DateTime, Local};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;
use sysinfo::System;

#[derive(Debug, Clone)]
pub struct PerformanceMetrics {
    pub operation_name: String,
    pub started_at: DateTime<Local>,
    pub duration_secs: f64,
    pub success: bool,
    pub error_message: String,
    pub metadata: HashMap<String, String>,
}

#[derive(Debug, Clone, Default)]
pub struct PerfStat {
    pub count: u64,
    pub success_count: u64,
    pub fail_count: u64,
    pub total_duration: f64,
    pub min_duration: f64,
    pub max_duration: f64,
    pub avg_duration: f64,
}

#[derive(Debug, Clone, Default)]
pub struct SystemMetrics {
    pub cpu_percent: f32,
    pub total_memory_mb: f64,
    pub used_memory_mb: f64,
    pub total_threads: usize,
}

#[derive(Debug)]
pub struct PerformanceMonitor {
    history_size: usize,
    history: VecDeque<PerformanceMetrics>,
    stats: HashMap<String, PerfStat>,
}

impl PerformanceMonitor {
    pub fn new(history_size: usize) -> Self {
        Self {
            history_size: history_size.max(1),
            history: VecDeque::with_capacity(history_size.max(1)),
            stats: HashMap::new(),
        }
    }

    pub fn start_operation(
        monitor: Arc<Mutex<Self>>,
        operation_name: impl Into<String>,
        metadata: HashMap<String, String>,
    ) -> OperationTimer {
        OperationTimer {
            monitor,
            operation_name: operation_name.into(),
            metadata,
            started_at: Local::now(),
            instant: Instant::now(),
            finished: false,
        }
    }

    pub fn record(&mut self, item: PerformanceMetrics) {
        if self.history.len() >= self.history_size {
            self.history.pop_front();
        }
        self.update_stats(&item);
        if item.duration_secs > 10.0 {
            if item.success {
                tracing::warn!(
                    "[性能] {}: {:.2}秒",
                    item.operation_name,
                    item.duration_secs
                );
            } else {
                tracing::warn!(
                    "[性能] {}: {:.2}秒 (失败: {})",
                    item.operation_name,
                    item.duration_secs,
                    item.error_message
                );
            }
        } else if !item.success {
            tracing::warn!(
                "[性能] {}: {:.2}秒 (失败: {})",
                item.operation_name,
                item.duration_secs,
                item.error_message
            );
        } else {
            tracing::debug!(
                "[性能] {}: {:.2}秒",
                item.operation_name,
                item.duration_secs
            );
        }
        self.history.push_back(item);
    }

    pub fn get_stats(&self, operation_name: Option<&str>) -> HashMap<String, PerfStat> {
        if let Some(name) = operation_name {
            let mut map = HashMap::new();
            if let Some(item) = self.stats.get(name) {
                map.insert(name.to_string(), item.clone());
            }
            return map;
        }
        self.stats.clone()
    }

    pub fn get_system_metrics(&self) -> SystemMetrics {
        let mut sys = System::new_all();
        sys.refresh_all();

        let total_memory_mb = sys.total_memory() as f64 / 1024.0 / 1024.0;
        let used_memory_mb = sys.used_memory() as f64 / 1024.0 / 1024.0;

        let cpu_percent = if sys.cpus().is_empty() {
            0.0
        } else {
            let sum: f32 = sys.cpus().iter().map(|item| item.cpu_usage()).sum();
            sum / (sys.cpus().len() as f32)
        };

        let total_threads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1);
        SystemMetrics {
            cpu_percent,
            total_memory_mb,
            used_memory_mb,
            total_threads,
        }
    }

    pub fn recent_metrics(
        &self,
        count: usize,
        operation_name: Option<&str>,
    ) -> Vec<PerformanceMetrics> {
        let wanted = count.max(1);
        let mut data: Vec<PerformanceMetrics> = self
            .history
            .iter()
            .filter(|item| {
                operation_name
                    .map(|name| item.operation_name == name)
                    .unwrap_or(true)
            })
            .cloned()
            .collect();
        if data.len() > wanted {
            data = data.split_off(data.len() - wanted);
        }
        data
    }

    pub fn generate_report(&self) -> String {
        let system = self.get_system_metrics();
        let mut lines = vec![
            "============================================================".to_string(),
            "性能监控报告".to_string(),
            "============================================================".to_string(),
            String::new(),
            format!("CPU使用率: {:.1}%", system.cpu_percent),
            format!("内存总量: {:.1}MB", system.total_memory_mb),
            format!("内存占用: {:.1}MB", system.used_memory_mb),
            format!("可用并发线程: {}", system.total_threads),
            String::new(),
            "操作统计:".to_string(),
        ];

        for (name, stat) in &self.stats {
            let success_rate = if stat.count > 0 {
                (stat.success_count as f64 / stat.count as f64) * 100.0
            } else {
                0.0
            };
            lines.push(format!("  操作: {}", name));
            lines.push(format!("    执行次数: {}", stat.count));
            lines.push(format!("    成功次数: {}", stat.success_count));
            lines.push(format!("    失败次数: {}", stat.fail_count));
            lines.push(format!("    成功率: {:.1}%", success_rate));
            lines.push(format!("    平均耗时: {:.2}秒", stat.avg_duration));
            lines.push(format!("    最短耗时: {:.2}秒", stat.min_duration));
            lines.push(format!("    最长耗时: {:.2}秒", stat.max_duration));
        }
        lines.push("============================================================".to_string());
        lines.join("\n")
    }

    fn update_stats(&mut self, item: &PerformanceMetrics) {
        let entry = self
            .stats
            .entry(item.operation_name.clone())
            .or_insert_with(|| PerfStat {
                min_duration: f64::MAX,
                ..PerfStat::default()
            });
        entry.count += 1;
        if item.success {
            entry.success_count += 1;
        } else {
            entry.fail_count += 1;
        }
        entry.total_duration += item.duration_secs;
        entry.min_duration = entry.min_duration.min(item.duration_secs);
        entry.max_duration = entry.max_duration.max(item.duration_secs);
        entry.avg_duration = entry.total_duration / entry.count as f64;
    }
}

pub struct OperationTimer {
    monitor: Arc<Mutex<PerformanceMonitor>>,
    operation_name: String,
    metadata: HashMap<String, String>,
    started_at: DateTime<Local>,
    instant: Instant,
    finished: bool,
}

impl OperationTimer {
    pub fn finish(mut self, success: bool, error_message: Option<String>) {
        if self.finished {
            return;
        }
        let item = PerformanceMetrics {
            operation_name: self.operation_name.clone(),
            started_at: self.started_at,
            duration_secs: self.instant.elapsed().as_secs_f64(),
            success,
            error_message: error_message.unwrap_or_default(),
            metadata: self.metadata.clone(),
        };
        if let Ok(mut guard) = self.monitor.lock() {
            guard.record(item);
        }
        self.finished = true;
    }
}

impl Drop for OperationTimer {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        let item = PerformanceMetrics {
            operation_name: self.operation_name.clone(),
            started_at: self.started_at,
            duration_secs: self.instant.elapsed().as_secs_f64(),
            success: true,
            error_message: String::new(),
            metadata: self.metadata.clone(),
        };
        if let Ok(mut guard) = self.monitor.lock() {
            guard.record(item);
        }
        self.finished = true;
    }
}

pub fn get_performance_monitor() -> Arc<Mutex<PerformanceMonitor>> {
    static GLOBAL_MONITOR: OnceLock<Arc<Mutex<PerformanceMonitor>>> = OnceLock::new();
    GLOBAL_MONITOR
        .get_or_init(|| Arc::new(Mutex::new(PerformanceMonitor::new(1000))))
        .clone()
}
