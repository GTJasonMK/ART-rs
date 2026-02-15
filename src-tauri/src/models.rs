use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

fn default_max_workers() -> usize {
    9
}

fn default_query_interval() -> u64 {
    60
}

fn default_retry_times() -> u32 {
    2
}

fn default_retry_delay() -> u64 {
    3
}

fn default_daily_rollover_hour() -> u32 {
    8
}

fn default_api_base_url() -> String {
    "https://anyrouter.top".to_string()
}

fn default_api_timeout() -> u64 {
    8
}

fn default_api_fallback_to_web() -> bool {
    true
}

fn default_log_level() -> String {
    "INFO".to_string()
}

fn default_log_file() -> String {
    "anyrouter_monitor.log".to_string()
}

fn default_web_timeout_seconds() -> u64 {
    90
}

fn default_web_pool_size() -> usize {
    4
}

fn default_web_pool_max_size() -> usize {
    9
}

fn default_browser_headless() -> bool {
    true
}

fn default_browser_timeout() -> u64 {
    20
}

fn default_browser_page_load_timeout() -> u64 {
    30
}

fn default_browser_implicitly_wait() -> u64 {
    2
}

fn default_browser_window_size() -> String {
    "1920,1080".to_string()
}

fn default_browser_disable_images() -> bool {
    true
}

fn default_browser_disable_javascript() -> bool {
    false
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceConfig {
    #[serde(default = "default_max_workers")]
    pub max_workers: usize,
    #[serde(default = "default_query_interval")]
    pub query_interval: u64,
    #[serde(default = "default_retry_times")]
    pub retry_times: u32,
    #[serde(default = "default_retry_delay")]
    pub retry_delay: u64,
    #[serde(default = "default_daily_rollover_hour")]
    pub daily_rollover_hour: u32,
}

impl Default for PerformanceConfig {
    fn default() -> Self {
        Self {
            max_workers: default_max_workers(),
            query_interval: default_query_interval(),
            retry_times: default_retry_times(),
            retry_delay: default_retry_delay(),
            daily_rollover_hour: default_daily_rollover_hour(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiConfig {
    #[serde(default = "default_api_base_url")]
    pub base_url: String,
    #[serde(default = "default_api_timeout")]
    pub timeout: u64,
    #[serde(default = "default_api_fallback_to_web")]
    pub fallback_to_web: bool,
}

impl Default for ApiConfig {
    fn default() -> Self {
        Self {
            base_url: default_api_base_url(),
            timeout: default_api_timeout(),
            fallback_to_web: default_api_fallback_to_web(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoggingConfig {
    #[serde(default = "default_log_level")]
    pub level: String,
    #[serde(default = "default_log_file")]
    pub file: String,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: default_log_level(),
            file: default_log_file(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserConfig {
    #[serde(default = "default_browser_headless")]
    pub headless: bool,
    #[serde(default = "default_browser_timeout")]
    pub timeout: u64,
    #[serde(default = "default_browser_page_load_timeout")]
    pub page_load_timeout: u64,
    #[serde(default = "default_browser_implicitly_wait")]
    pub implicitly_wait: u64,
    #[serde(default = "default_browser_window_size")]
    pub window_size: String,
    #[serde(default)]
    pub user_agent: Option<String>,
    #[serde(default = "default_browser_disable_images")]
    pub disable_images: bool,
    #[serde(default = "default_browser_disable_javascript")]
    pub disable_javascript: bool,
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            headless: default_browser_headless(),
            timeout: default_browser_timeout(),
            page_load_timeout: default_browser_page_load_timeout(),
            implicitly_wait: default_browser_implicitly_wait(),
            window_size: default_browser_window_size(),
            user_agent: None,
            disable_images: default_browser_disable_images(),
            disable_javascript: default_browser_disable_javascript(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebCheckConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default = "default_web_timeout_seconds")]
    pub timeout_seconds: u64,
    #[serde(default)]
    pub chromedriver_path: String,
    #[serde(default = "default_web_pool_size")]
    pub pool_size: usize,
    #[serde(default = "default_web_pool_max_size")]
    pub max_pool_size: usize,
}

impl Default for WebCheckConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            command: String::new(),
            args: Vec::new(),
            timeout_seconds: default_web_timeout_seconds(),
            chromedriver_path: String::new(),
            pool_size: default_web_pool_size(),
            max_pool_size: default_web_pool_max_size(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppConfig {
    #[serde(default)]
    pub browser: BrowserConfig,
    #[serde(default)]
    pub performance: PerformanceConfig,
    #[serde(default)]
    pub api: ApiConfig,
    #[serde(default)]
    pub logging: LoggingConfig,
    #[serde(default)]
    pub web_check: WebCheckConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub username: String,
    pub password: String,
    pub api_key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BalanceCacheRecord {
    #[serde(default)]
    pub balance: String,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub apikey_sync_success: Option<bool>,
    #[serde(default)]
    pub apikey_sync_message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct BalanceCacheFile {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub accounts: BTreeMap<String, BalanceCacheRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DailyWebStateFile {
    #[serde(default)]
    pub version: u32,
    #[serde(default)]
    pub updated_at: String,
    #[serde(default)]
    pub accounts: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckResult {
    pub username: String,
    pub success: bool,
    pub balance_text: String,
    pub source: String,
    pub message: String,
}

/// 实时进度日志事件载荷
#[derive(Debug, Clone, Serialize)]
pub struct ProgressEvent {
    /// 日志级别: "info" / "warn" / "error" / "success"
    pub level: String,
    /// 关联的账号名，批次级日志为空字符串
    pub username: String,
    /// 日志正文
    pub message: String,
}
