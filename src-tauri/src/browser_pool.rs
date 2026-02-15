use crate::driver_manager::get_chromedriver_path;
use crate::models::WebCheckConfig;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::sleep;
use std::time::{Duration, Instant, SystemTime};

#[derive(Debug)]
struct DriverProcess {
    id: String,
    port: u16,
    url: String,
    child: Child,
    created_at: SystemTime,
    last_used: SystemTime,
    use_count: u64,
    is_busy: bool,
}

impl DriverProcess {
    fn is_alive(&mut self) -> bool {
        if let Ok(Some(_)) = self.child.try_wait() {
            return false;
        }
        TcpStream::connect(("127.0.0.1", self.port)).is_ok()
    }

    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[derive(Debug, Clone)]
pub struct PoolTicket {
    pub url: String,
    index: usize,
}

#[derive(Debug)]
pub struct BrowserPool {
    pool_size: usize,
    max_pool_size: usize,
    processes: Vec<DriverProcess>,
    stats: HashMap<String, f64>,
    chromedriver_path: PathBuf,
}

impl BrowserPool {
    pub fn new(config: &WebCheckConfig) -> Result<Self> {
        let path = if !config.chromedriver_path.trim().is_empty() {
            PathBuf::from(config.chromedriver_path.trim())
        } else {
            get_chromedriver_path()?
        };

        let mut pool = Self {
            pool_size: config.pool_size.max(1),
            max_pool_size: config.max_pool_size.max(1),
            processes: Vec::new(),
            stats: HashMap::from([
                ("total_created".to_string(), 0.0),
                ("total_reused".to_string(), 0.0),
                ("total_requests".to_string(), 0.0),
            ]),
            chromedriver_path: path,
        };
        pool.pool_size = pool.pool_size.min(pool.max_pool_size);
        pool.init_pool()?;
        Ok(pool)
    }

    fn init_pool(&mut self) -> Result<()> {
        tracing::info!("初始化浏览器池: size={}", self.pool_size);
        for i in 0..self.pool_size {
            let id = format!("browser_{i}");
            if let Ok(process) = self.create_process(&id) {
                self.processes.push(process);
            } else {
                tracing::warn!("浏览器池预创建失败: id={}", id);
            }
        }
        Ok(())
    }

    fn create_process(&mut self, id: &str) -> Result<DriverProcess> {
        let port = find_free_port()?;
        let mut cmd = Command::new(&self.chromedriver_path);
        cmd.arg(format!("--port={port}"))
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }
        let child = cmd.spawn().with_context(|| {
            format!(
                "启动 chromedriver 失败: path={}",
                self.chromedriver_path.display()
            )
        })?;

        wait_port_ready(port, Duration::from_secs(8))
            .with_context(|| format!("等待 chromedriver 端口就绪失败: {port}"))?;

        let now = SystemTime::now();
        let process = DriverProcess {
            id: id.to_string(),
            port,
            url: format!("http://127.0.0.1:{port}"),
            child,
            created_at: now,
            last_used: now,
            use_count: 0,
            is_busy: false,
        };
        *self.stats.entry("total_created".to_string()).or_default() += 1.0;
        Ok(process)
    }

    /// 尝试获取一个可用的浏览器实例（非阻塞）。
    /// 返回 Ok(Some(ticket)) 表示成功获取，Ok(None) 表示当前无可用实例。
    /// 调用方应在获取失败时释放锁后等待重试，避免持锁睡眠导致死锁。
    pub fn try_acquire(&mut self) -> Result<Option<PoolTicket>> {
        self.remove_dead_processes();

        // 尝试复用已有空闲实例
        for (idx, item) in self.processes.iter_mut().enumerate() {
            if item.is_busy {
                continue;
            }
            if !item.is_alive() {
                continue;
            }
            item.is_busy = true;
            item.use_count += 1;
            item.last_used = SystemTime::now();
            *self.stats.entry("total_reused".to_string()).or_default() += 1.0;
            *self.stats.entry("total_requests".to_string()).or_default() += 1.0;
            return Ok(Some(PoolTicket {
                url: item.url.clone(),
                index: idx,
            }));
        }

        // 尝试创建新实例（未达上限时）
        if self.processes.len() < self.max_pool_size {
            let id = format!("browser_{}", self.processes.len());
            let mut process = self.create_process(&id)?;
            process.is_busy = true;
            process.use_count += 1;
            process.last_used = SystemTime::now();
            let idx = self.processes.len();
            self.processes.push(process);
            *self.stats.entry("total_requests".to_string()).or_default() += 1.0;
            return Ok(Some(PoolTicket {
                url: self.processes[idx].url.clone(),
                index: idx,
            }));
        }

        // 所有实例都在使用中且已达上限
        Ok(None)
    }

    pub fn release(&mut self, ticket: PoolTicket) {
        if let Some(item) = self.processes.get_mut(ticket.index) {
            item.is_busy = false;
            item.last_used = SystemTime::now();
        }
    }

    pub fn get_stats(&self) -> HashMap<String, f64> {
        let mut data = self.stats.clone();
        let pool_size = self.processes.len() as f64;
        let busy_count = self.processes.iter().filter(|item| item.is_busy).count() as f64;
        // alive_count 使用进程总数，已死亡的进程会在下次 try_acquire 时清理
        let alive_count = pool_size;
        data.insert("pool_size".to_string(), pool_size);
        data.insert("busy_count".to_string(), busy_count);
        data.insert("alive_count".to_string(), alive_count);
        data.insert(
            "available_count".to_string(),
            (pool_size - busy_count).max(0.0),
        );
        let total_reused = *data.get("total_reused").unwrap_or(&0.0);
        let total_requests = *data.get("total_requests").unwrap_or(&0.0);
        data.insert(
            "reuse_rate".to_string(),
            if total_requests > 0.0 {
                (total_reused / total_requests) * 100.0
            } else {
                0.0
            },
        );
        data
    }

    pub fn shutdown(&mut self) {
        for item in &mut self.processes {
            item.kill();
        }
        self.processes.clear();
    }

    fn remove_dead_processes(&mut self) {
        let mut kept = Vec::new();
        for mut item in self.processes.drain(..) {
            if item.is_busy {
                kept.push(item);
                continue;
            }
            if item.is_alive() {
                kept.push(item);
            } else {
                tracing::warn!("移除失效浏览器进程: id={}", item.id);
                item.kill();
            }
        }
        self.processes = kept;
    }
}

impl Drop for BrowserPool {
    fn drop(&mut self) {
        self.shutdown();
    }
}

pub fn get_global_pool(config: &WebCheckConfig) -> Result<Arc<Mutex<BrowserPool>>> {
    if let Some(pool) = GLOBAL_POOL.get() {
        return Ok(pool.clone());
    }
    let created = Arc::new(Mutex::new(BrowserPool::new(config)?));
    let _ = GLOBAL_POOL.set(created.clone());
    Ok(created)
}

pub fn shutdown_global_pool() {
    if let Some(pool) = GLOBAL_POOL.get() {
        if let Ok(mut guard) = pool.lock() {
            guard.shutdown();
            tracing::info!("已执行全局浏览器池清理");
        }
    }
}

fn find_free_port() -> Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0").with_context(|| "申请临时端口失败")?;
    let port = listener
        .local_addr()
        .with_context(|| "读取端口失败")?
        .port();
    Ok(port)
}

fn wait_port_ready(port: u16, timeout: Duration) -> Result<()> {
    let started = Instant::now();
    while started.elapsed() < timeout {
        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return Ok(());
        }
        sleep(Duration::from_millis(120));
    }
    anyhow::bail!("端口未就绪: {port}")
}
static GLOBAL_POOL: OnceLock<Arc<Mutex<BrowserPool>>> = OnceLock::new();
