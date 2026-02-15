use crate::models::{BalanceCacheFile, BalanceCacheRecord, DailyWebStateFile};
use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Local, NaiveDate, NaiveDateTime, Timelike};
use serde_json::Value;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct StateStore {
    balance_cache_file: PathBuf,
    daily_web_state_file: PathBuf,
    balance_cache: BTreeMap<String, BalanceCacheRecord>,
    daily_web_state: BTreeMap<String, String>,
    daily_rollover_hour: u32,
}

impl StateStore {
    pub fn load(
        balance_cache_file: PathBuf,
        daily_web_state_file: PathBuf,
        daily_rollover_hour: u32,
    ) -> Result<Self> {
        let mut store = Self {
            balance_cache_file,
            daily_web_state_file,
            balance_cache: BTreeMap::new(),
            daily_web_state: BTreeMap::new(),
            daily_rollover_hour: if daily_rollover_hour <= 23 {
                daily_rollover_hour
            } else {
                8
            },
        };
        store.load_balance_cache()?;
        store.load_daily_web_state()?;
        Ok(store)
    }

    fn load_balance_cache(&mut self) -> Result<()> {
        if !self.balance_cache_file.exists() {
            return Ok(());
        }
        let raw = fs::read_to_string(&self.balance_cache_file).with_context(|| {
            format!(
                "读取余额缓存文件失败: {}",
                self.balance_cache_file.display()
            )
        })?;
        let parsed: Value =
            serde_json::from_str(&raw).with_context(|| "解析 balance_cache.json 失败")?;
        self.balance_cache = parse_balance_cache_accounts(&parsed);
        Ok(())
    }

    fn load_daily_web_state(&mut self) -> Result<()> {
        if !self.daily_web_state_file.exists() {
            return Ok(());
        }
        let raw = fs::read_to_string(&self.daily_web_state_file).with_context(|| {
            format!(
                "读取每日首查状态文件失败: {}",
                self.daily_web_state_file.display()
            )
        })?;
        let parsed: Value =
            serde_json::from_str(&raw).with_context(|| "解析 daily_web_login_state.json 失败")?;
        let (state_map, updated_at) = parse_daily_web_accounts(&parsed);
        self.daily_web_state = state_map;

        // 兼容修正：旧版按00:00切日，若记录写入时间在切日前且值为“当天”，回拨一天
        if !updated_at.is_empty() {
            if let Some((hour, old_day)) = parse_updated_time(&updated_at) {
                if hour < self.daily_rollover_hour {
                    let new_day = old_day - Duration::days(1);
                    let mut corrected = 0usize;
                    for value in self.daily_web_state.values_mut() {
                        if *value == old_day.to_string() {
                            *value = new_day.to_string();
                            corrected += 1;
                        }
                    }
                    if corrected > 0 {
                        tracing::warn!(
                            "检测到旧版午夜切日状态，已按{:02}:00规则修正 {} 条",
                            self.daily_rollover_hour,
                            corrected
                        );
                        self.save_daily_web_state()?;
                    }
                }
            }
        }

        Ok(())
    }

    pub fn save_balance_cache(&self) -> Result<()> {
        let payload = BalanceCacheFile {
            version: 1,
            updated_at: Local::now().to_rfc3339(),
            accounts: self.balance_cache.clone(),
        };
        write_json_file(&self.balance_cache_file, &payload)
    }

    pub fn save_daily_web_state(&self) -> Result<()> {
        let payload = DailyWebStateFile {
            version: 1,
            updated_at: Local::now().to_rfc3339(),
            accounts: self.daily_web_state.clone(),
        };
        write_json_file(&self.daily_web_state_file, &payload)
    }

    pub fn current_cycle_day(&self) -> NaiveDate {
        let now = Local::now();
        if now.hour() < self.daily_rollover_hour {
            (now - Duration::days(1)).date_naive()
        } else {
            now.date_naive()
        }
    }

    pub fn should_force_web_query(&self, username: &str) -> bool {
        let cycle_day = self.current_cycle_day().to_string();
        let last_day = self
            .daily_web_state
            .get(username)
            .map(|v| v.as_str())
            .unwrap_or_default();
        cycle_day != last_day
    }

    pub fn mark_web_query_success(&mut self, username: &str) -> Result<()> {
        let cycle_day = self.current_cycle_day().to_string();
        self.daily_web_state
            .insert(username.to_string(), cycle_day.clone());
        self.save_daily_web_state()?;
        tracing::debug!("账号 {} 已记录网页登录成功周期日: {}", username, cycle_day);
        Ok(())
    }

    pub fn update_balance_cache(
        &mut self,
        username: &str,
        balance: &str,
        apikey_sync_success: Option<bool>,
        apikey_sync_message: Option<&str>,
    ) -> Result<()> {
        let mut record = self
            .balance_cache
            .get(username)
            .cloned()
            .unwrap_or_default();
        record.balance = balance.to_string();
        record.updated_at = Local::now().to_rfc3339();
        record.apikey_sync_success = apikey_sync_success;
        if let Some(msg) = apikey_sync_message {
            record.apikey_sync_message = msg.to_string();
        }
        self.balance_cache.insert(username.to_string(), record);
        self.save_balance_cache()
    }

    pub fn get_cached_balance_text(&self, username: &str) -> Option<String> {
        self.balance_cache
            .get(username)
            .map(|item| item.balance.clone())
            .filter(|item| !item.trim().is_empty())
    }

    pub fn get_cached_balance_record(&self, username: &str) -> Option<BalanceCacheRecord> {
        self.balance_cache.get(username).cloned()
    }
}

fn write_json_file<T: serde::Serialize>(path: &Path, value: &T) -> Result<()> {
    let tmp = path.with_extension("tmp");
    let content = serde_json::to_string_pretty(value).with_context(|| "序列化JSON失败")?;
    fs::write(&tmp, content).with_context(|| format!("写入临时文件失败: {}", tmp.display()))?;
    fs::rename(&tmp, path).with_context(|| format!("原子替换失败: {}", path.display()))?;
    Ok(())
}

fn parse_updated_time(text: &str) -> Option<(u32, NaiveDate)> {
    if let Ok(with_tz) = DateTime::parse_from_rfc3339(text) {
        return Some((with_tz.hour(), with_tz.date_naive()));
    }
    if let Ok(no_tz) = NaiveDateTime::parse_from_str(text, "%Y-%m-%dT%H:%M:%S") {
        return Some((no_tz.hour(), no_tz.date()));
    }
    None
}

fn parse_balance_cache_accounts(root: &Value) -> BTreeMap<String, BalanceCacheRecord> {
    let mut normalized = BTreeMap::new();
    let account_map = root
        .get("accounts")
        .and_then(Value::as_object)
        .or_else(|| root.as_object());

    if let Some(accounts) = account_map {
        for (username, item) in accounts {
            let mut record = BalanceCacheRecord::default();
            match item {
                Value::Object(obj) => {
                    record.balance = value_to_text(obj.get("balance")).trim().to_string();
                    record.updated_at = value_to_text(obj.get("updated_at")).trim().to_string();
                    record.apikey_sync_success =
                        obj.get("apikey_sync_success").and_then(Value::as_bool);
                    record.apikey_sync_message = value_to_text(obj.get("apikey_sync_message"))
                        .trim()
                        .to_string();
                }
                _ => {
                    record.balance = value_to_text(Some(item)).trim().to_string();
                }
            }
            if !record.balance.is_empty() {
                normalized.insert(username.to_string(), record);
            }
        }
    }

    normalized
}

fn parse_daily_web_accounts(root: &Value) -> (BTreeMap<String, String>, String) {
    let mut normalized = BTreeMap::new();
    let updated_at = root
        .get("updated_at")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .to_string();

    let account_map = root
        .get("accounts")
        .and_then(Value::as_object)
        .or_else(|| root.as_object());

    if let Some(accounts) = account_map {
        for (username, value) in accounts {
            let day = value_to_text(Some(value)).trim().to_string();
            if is_valid_day_text(&day) {
                normalized.insert(username.to_string(), day);
            }
        }
    }

    (normalized, updated_at)
}

fn value_to_text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.to_string(),
        Some(Value::Number(num)) => num.to_string(),
        Some(Value::Bool(v)) => v.to_string(),
        _ => String::new(),
    }
}

fn is_valid_day_text(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.len() == 10
        && bytes.get(4) == Some(&b'-')
        && bytes.get(7) == Some(&b'-')
        && bytes
            .iter()
            .enumerate()
            .all(|(idx, ch)| idx == 4 || idx == 7 || ch.is_ascii_digit())
}
