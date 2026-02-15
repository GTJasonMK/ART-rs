#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod api_client;
mod browser_pool;
mod config;
mod driver_manager;
mod models;
mod monitor;
mod performance_monitor;
mod state;
mod utils;
mod web_check;
mod web_native;

use anyhow::{Context, Result};
use chrono::Local;
use config::{RuntimeFiles, load_accounts, load_app_config, save_accounts};
use models::{Account, AppConfig, CheckResult};
use serde::Serialize;
use serde_json::{Map, Value};
use state::StateStore;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Instant;
use tauri::State;
use tokio::sync::{Mutex, RwLock};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::prelude::*;

#[derive(Debug)]
struct AppState {
    files: RuntimeFiles,
    config: Arc<RwLock<AppConfig>>,
    accounts: Arc<RwLock<Vec<Account>>>,
    state_store: Arc<Mutex<StateStore>>,
    query_lock: Mutex<()>,
}

#[derive(Debug, Clone, Serialize)]
struct AppSnapshot {
    config_dir: String,
    query_interval: u64,
    daily_rollover_hour: u32,
    fallback_to_web: bool,
    accounts: Vec<Account>,
    cached_results: Vec<CheckResult>,
}

#[derive(Debug, Clone, Serialize)]
struct ActionResponse {
    success: bool,
    message: String,
    accounts: Vec<Account>,
}

#[derive(Debug, Clone, Serialize)]
struct QueryResponse {
    results: Vec<CheckResult>,
    elapsed_secs: f64,
    finished_at: String,
    success_count: usize,
    fail_count: usize,
    total_balance: f64,
    total_balance_count: usize,
}

fn main() {
    if let Err(err) = run_app() {
        eprintln!("启动失败: {err}");
    }
}

fn run_app() -> Result<()> {
    let config_dir = resolve_config_dir();
    let files = RuntimeFiles::new(config_dir);
    let config = load_app_config(&files.config_file)?;
    let log_path = resolve_log_path(&files, &config);
    init_logger(&config.logging.level, &log_path)?;

    tracing::info!("ART-rs Tauri 启动");
    tracing::info!("配置目录: {}", files.config_dir.display());
    tracing::info!(
        "每日网页登录切日时间: {:02}:00",
        config.performance.daily_rollover_hour
    );

    let mut accounts = load_accounts(&files.credentials_file)?;
    sort_accounts(&mut accounts);
    tracing::info!("成功加载 {} 个账号", accounts.len());
    if accounts.is_empty() {
        tracing::warn!(
            "未读取到账号，请检查账号文件: path={}, exists={}",
            files.credentials_file.display(),
            files.credentials_file.exists()
        );
    }

    let state_store = StateStore::load(
        files.balance_cache_file.clone(),
        files.daily_web_state_file.clone(),
        config.performance.daily_rollover_hour,
    )
    .with_context(|| "初始化状态存储失败")?;

    let app_state = AppState {
        files,
        config: Arc::new(RwLock::new(config)),
        accounts: Arc::new(RwLock::new(accounts)),
        state_store: Arc::new(Mutex::new(state_store)),
        query_lock: Mutex::new(()),
    };

    let app = tauri::Builder::default()
        .manage(app_state)
        .invoke_handler(tauri::generate_handler![
            get_snapshot_command,
            reload_accounts_command,
            upsert_account_command,
            remove_account_command,
            query_balances_command,
            web_login_only_command,
            get_cached_results_command,
            save_claude_token_command,
            save_openai_key_command,
            performance_report_command,
            get_current_claude_account_command
        ])
        .build(tauri::generate_context!())
        .with_context(|| "构建 Tauri 应用失败")?;

    app.run(|_app, _event| {});
    browser_pool::shutdown_global_pool();
    Ok(())
}

#[tauri::command]
async fn get_snapshot_command(state: State<'_, AppState>) -> Result<AppSnapshot, String> {
    let config = state.config.read().await.clone();
    let accounts = state.accounts.read().await.clone();
    let cached_results = build_cached_results(&accounts, state.state_store.clone()).await;
    Ok(AppSnapshot {
        config_dir: state.files.config_dir.to_string_lossy().to_string(),
        query_interval: config.performance.query_interval.max(1),
        daily_rollover_hour: config.performance.daily_rollover_hour,
        fallback_to_web: config.api.fallback_to_web,
        accounts,
        cached_results,
    })
}

#[tauri::command]
async fn reload_accounts_command(state: State<'_, AppState>) -> Result<ActionResponse, String> {
    let accounts = reload_accounts_from_disk(&state).await?;
    Ok(ActionResponse {
        success: true,
        message: format!("已重新加载 {} 个账号", accounts.len()),
        accounts,
    })
}

#[tauri::command]
async fn upsert_account_command(
    state: State<'_, AppState>,
    username: String,
    password: String,
    api_key: Option<String>,
) -> Result<ActionResponse, String> {
    let username = username.trim().to_string();
    let password = password.trim().to_string();
    let api_key = api_key.unwrap_or_default().trim().to_string();
    if username.is_empty() || password.is_empty() {
        return Err("用户名和密码不能为空".to_string());
    }

    let mut guard = state.accounts.write().await;
    let mut accounts = guard.clone();
    let mut replaced = false;
    for item in &mut accounts {
        if item.username == username {
            item.password = password.clone();
            item.api_key = api_key.clone();
            replaced = true;
            break;
        }
    }
    if !replaced {
        accounts.push(Account {
            username: username.clone(),
            password,
            api_key,
        });
    }
    sort_accounts(&mut accounts);

    save_accounts(&state.files.credentials_file, &accounts)
        .map_err(|e| format!("写入账号文件失败: {e}"))?;
    *guard = accounts.clone();

    Ok(ActionResponse {
        success: true,
        message: if replaced {
            format!("已更新账号: {username}")
        } else {
            format!("已新增账号: {username}")
        },
        accounts,
    })
}

#[tauri::command]
async fn remove_account_command(
    state: State<'_, AppState>,
    username: String,
) -> Result<ActionResponse, String> {
    let username = username.trim().to_string();
    if username.is_empty() {
        return Err("账号名不能为空".to_string());
    }
    let mut guard = state.accounts.write().await;
    let before_len = guard.len();
    let mut accounts = guard.clone();
    accounts.retain(|item| item.username != username);
    if accounts.len() == before_len {
        return Ok(ActionResponse {
            success: false,
            message: format!("未找到账号: {username}"),
            accounts: guard.clone(),
        });
    }

    save_accounts(&state.files.credentials_file, &accounts)
        .map_err(|e| format!("删除账号失败: {e}"))?;
    *guard = accounts.clone();
    Ok(ActionResponse {
        success: true,
        message: format!("已删除账号: {username}"),
        accounts,
    })
}

#[tauri::command]
async fn query_balances_command(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    target_username: Option<String>,
) -> Result<QueryResponse, String> {
    let _query_guard = state.query_lock.lock().await;

    let accounts = state.accounts.read().await.clone();
    let config = state.config.read().await.clone();
    let started = Instant::now();
    let results = monitor::check_accounts(
        accounts,
        config,
        state.state_store.clone(),
        target_username.map(|item| item.trim().to_string()),
        app,
    )
    .await;

    let elapsed_secs = started.elapsed().as_secs_f64();
    let success_count = results.iter().filter(|item| item.success).count();
    let fail_count = results.len().saturating_sub(success_count);
    let (total_balance, total_balance_count) = calculate_total_balance(&results);
    Ok(QueryResponse {
        results,
        elapsed_secs,
        finished_at: Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        success_count,
        fail_count,
        total_balance,
        total_balance_count,
    })
}

#[tauri::command]
async fn web_login_only_command(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    target_username: Option<String>,
) -> Result<QueryResponse, String> {
    let _query_guard = state.query_lock.lock().await;

    let accounts = state.accounts.read().await.clone();
    let config = state.config.read().await.clone();
    let started = Instant::now();
    let results = monitor::check_accounts_web_only(
        accounts,
        config,
        state.state_store.clone(),
        target_username.map(|item| item.trim().to_string()),
        app,
    )
    .await;

    let elapsed_secs = started.elapsed().as_secs_f64();
    let success_count = results.iter().filter(|item| item.success).count();
    let fail_count = results.len().saturating_sub(success_count);
    let (total_balance, total_balance_count) = calculate_total_balance(&results);
    Ok(QueryResponse {
        results,
        elapsed_secs,
        finished_at: Local::now().format("%Y-%m-%d %H:%M:%S").to_string(),
        success_count,
        fail_count,
        total_balance,
        total_balance_count,
    })
}

#[tauri::command]
async fn get_cached_results_command(
    state: State<'_, AppState>,
) -> Result<Vec<CheckResult>, String> {
    let accounts = state.accounts.read().await.clone();
    Ok(build_cached_results(&accounts, state.state_store.clone()).await)
}

#[tauri::command]
async fn save_claude_token_command(
    state: State<'_, AppState>,
    username: String,
) -> Result<String, String> {
    let key = find_account_api_key(&state, username.trim()).await?;
    let path = save_claude_token(&key).map_err(|e| format!("写入 Claude Token 失败: {e}"))?;
    Ok(format!("已写入 Claude Token: {}", path.display()))
}

#[tauri::command]
async fn save_openai_key_command(
    state: State<'_, AppState>,
    username: String,
) -> Result<String, String> {
    let key = find_account_api_key(&state, username.trim()).await?;
    save_openai_key(&key).map_err(|e| format!("写入 OpenAI Key 失败: {e}"))
}

#[tauri::command]
fn performance_report_command() -> String {
    let monitor = performance_monitor::get_performance_monitor();
    match monitor.lock() {
        Ok(guard) => guard.generate_report(),
        Err(_) => "性能监控状态不可用".to_string(),
    }
}

#[tauri::command]
async fn get_current_claude_account_command(
    state: State<'_, AppState>,
) -> Result<String, String> {
    let token = read_current_claude_token().unwrap_or_default();
    if token.is_empty() {
        return Ok(String::new());
    }
    let accounts = state.accounts.read().await;
    for account in accounts.iter() {
        if !account.api_key.is_empty() && account.api_key == token {
            return Ok(account.username.clone());
        }
    }
    Ok(String::new())
}

async fn find_account_api_key(
    state: &State<'_, AppState>,
    username: &str,
) -> Result<String, String> {
    if username.trim().is_empty() {
        return Err("账号名不能为空".to_string());
    }
    let account = state
        .accounts
        .read()
        .await
        .iter()
        .find(|item| item.username == username)
        .cloned();
    let Some(account) = account else {
        return Err(format!("未找到账号: {username}"));
    };
    if account.api_key.trim().is_empty() {
        return Err(format!("账号 {username} 未配置 API Key"));
    }
    Ok(account.api_key)
}

async fn reload_accounts_from_disk(state: &State<'_, AppState>) -> Result<Vec<Account>, String> {
    let mut guard = state.accounts.write().await;
    let mut accounts = load_accounts(&state.files.credentials_file)
        .map_err(|e| format!("读取账号文件失败: {e}"))?;
    sort_accounts(&mut accounts);
    *guard = accounts.clone();
    Ok(accounts)
}

async fn build_cached_results(
    accounts: &[Account],
    state_store: Arc<Mutex<StateStore>>,
) -> Vec<CheckResult> {
    let guard = state_store.lock().await;
    let mut results = Vec::with_capacity(accounts.len());
    for account in accounts {
        if let Some(record) = guard.get_cached_balance_record(&account.username) {
            results.push(CheckResult {
                username: account.username.clone(),
                success: true,
                balance_text: record.balance.clone(),
                source: "cache".to_string(),
                message: if record.updated_at.trim().is_empty() {
                    "缓存余额".to_string()
                } else {
                    format!("缓存更新时间: {}", record.updated_at)
                },
            });
        } else {
            results.push(CheckResult {
                username: account.username.clone(),
                success: false,
                balance_text: "等待".to_string(),
                source: "-".to_string(),
                message: "待机".to_string(),
            });
        }
    }
    results
}

fn calculate_total_balance(results: &[CheckResult]) -> (f64, usize) {
    let mut total = 0.0_f64;
    let mut count = 0_usize;
    for row in results {
        if !row.success {
            continue;
        }
        if let Some(value) = utils::parse_first_number(&row.balance_text) {
            total += value;
            count += 1;
        }
    }
    (total, count)
}


fn sort_accounts(accounts: &mut [Account]) {
    accounts.sort_by(|a, b| a.username.cmp(&b.username));
}

fn resolve_config_dir() -> PathBuf {
    if let Some(arg_path) = parse_config_dir_from_args() {
        return arg_path;
    }
    if let Ok(raw) = std::env::var("ART_RS_CONFIG_DIR") {
        let text = raw.trim();
        if !text.is_empty() {
            return PathBuf::from(text);
        }
    }

    let mut seeds = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        seeds.push(cwd);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            seeds.push(parent.to_path_buf());
        }
    }
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    seeds.push(manifest_dir.clone());
    if let Some(parent) = manifest_dir.parent() {
        seeds.push(parent.to_path_buf());
    }

    let mut visited = std::collections::HashSet::new();
    for dir in seeds {
        let normalized = dir.to_string_lossy().to_string();
        if !visited.insert(normalized) {
            continue;
        }
        if has_runtime_files(&dir) {
            tracing::info!("已自动定位配置目录: {}", dir.display());
            return dir;
        }
    }

    let fallback = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    tracing::warn!(
        "未定位到配置目录（config.json/credentials.txt），回退当前目录: {}",
        fallback.display()
    );
    fallback
}

fn has_runtime_files(dir: &Path) -> bool {
    dir.join("credentials.txt").exists() || dir.join("config.json").exists()
}

fn parse_config_dir_from_args() -> Option<PathBuf> {
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--config-dir" {
            if let Some(value) = args.next() {
                return Some(PathBuf::from(value));
            }
            return None;
        }
    }
    None
}

fn resolve_log_path(files: &RuntimeFiles, config: &AppConfig) -> PathBuf {
    let candidate = PathBuf::from(config.logging.file.trim());
    if candidate.is_absolute() {
        candidate
    } else {
        files.config_dir.join(candidate)
    }
}

fn init_logger(level: &str, log_path: &Path) -> Result<()> {
    let directive = match level.to_ascii_uppercase().as_str() {
        "TRACE" => "trace",
        "DEBUG" => "debug",
        "WARN" => "warn",
        "ERROR" => "error",
        _ => "info",
    };
    let env_filter = EnvFilter::try_new(directive).with_context(|| "初始化日志级别失败")?;
    let log_dir = log_path
        .parent()
        .filter(|item| !item.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(log_dir).with_context(|| {
        format!(
            "创建日志目录失败: {}",
            log_dir.as_os_str().to_string_lossy()
        )
    })?;
    let log_file_name = log_path
        .file_name()
        .and_then(|item| item.to_str())
        .filter(|item| !item.trim().is_empty())
        .unwrap_or("anyrouter_monitor.log");

    let file_appender = tracing_appender::rolling::never(log_dir, log_file_name);
    let (file_writer, guard) = tracing_appender::non_blocking(file_appender);
    let _ = Box::leak(Box::new(guard));

    let stdout_layer = tracing_subscriber::fmt::layer()
        .with_target(true)
        .with_thread_names(true);
    let file_layer = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_target(true)
        .with_thread_names(true)
        .with_writer(file_writer);

    tracing_subscriber::registry()
        .with(env_filter)
        .with(stdout_layer)
        .with(file_layer)
        .try_init()
        .with_context(|| "初始化日志订阅器失败")?;

    tracing::info!("日志文件: {}", log_path.display());
    Ok(())
}

fn home_dir() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .with_context(|| "未找到用户目录")
}

fn read_current_claude_token() -> Result<String> {
    let home = home_dir()?;
    let target = home.join(".claude").join("settings.json");
    if !target.exists() {
        return Ok(String::new());
    }
    let raw = std::fs::read_to_string(&target)
        .with_context(|| format!("读取配置失败: {}", target.display()))?;
    let root: Value = serde_json::from_str(&raw).unwrap_or(Value::Null);
    let token = root
        .get("env")
        .and_then(|env| env.get("ANTHROPIC_AUTH_TOKEN"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string();
    Ok(token)
}

fn save_claude_token(token: &str) -> Result<PathBuf> {
    let home = home_dir()?;
    let target = home.join(".claude").join("settings.json");
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("创建目录失败: {}", parent.display()))?;
    }

    let mut root = if target.exists() {
        let raw = std::fs::read_to_string(&target)
            .with_context(|| format!("读取配置失败: {}", target.display()))?;
        serde_json::from_str::<Value>(&raw).unwrap_or_else(|_| Value::Object(Map::new()))
    } else {
        Value::Object(Map::new())
    };

    if !root.is_object() {
        root = Value::Object(Map::new());
    }
    let obj = root.as_object_mut().expect("root object");
    let env_value = obj
        .entry("env".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !env_value.is_object() {
        *env_value = Value::Object(Map::new());
    }
    let env_obj = env_value.as_object_mut().expect("env object");
    env_obj.insert(
        "ANTHROPIC_AUTH_TOKEN".to_string(),
        Value::String(token.to_string()),
    );

    let content = serde_json::to_string_pretty(&root).with_context(|| "序列化配置失败")?;
    std::fs::write(&target, content)
        .with_context(|| format!("写入配置失败: {}", target.display()))?;
    Ok(target)
}

fn save_openai_key_local(token: &str) -> Result<PathBuf> {
    let home = home_dir()?;
    let target = home.join(".codex").join("auth.json");
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("创建目录失败: {}", parent.display()))?;
    }

    let mut root = if target.exists() {
        let raw = std::fs::read_to_string(&target)
            .with_context(|| format!("读取配置失败: {}", target.display()))?;
        serde_json::from_str::<Value>(&raw).unwrap_or_else(|_| Value::Object(Map::new()))
    } else {
        Value::Object(Map::new())
    };

    if !root.is_object() {
        root = Value::Object(Map::new());
    }
    let obj = root.as_object_mut().expect("root object");
    obj.insert(
        "OPENAI_API_KEY".to_string(),
        Value::String(token.to_string()),
    );

    let content = serde_json::to_string_pretty(&root).with_context(|| "序列化配置失败")?;
    std::fs::write(&target, content)
        .with_context(|| format!("写入配置失败: {}", target.display()))?;
    Ok(target)
}

fn decode_wsl_output(bytes: &[u8]) -> String {
    if bytes.len() >= 2 && bytes[1] == 0 {
        let mut u16s = Vec::with_capacity(bytes.len() / 2);
        for chunk in bytes.chunks_exact(2) {
            u16s.push(u16::from_le_bytes([chunk[0], chunk[1]]));
        }
        if let Ok(text) = String::from_utf16(&u16s) {
            return text;
        }
    }
    String::from_utf8_lossy(bytes).to_string()
}

fn discover_wsl_distros() -> Vec<String> {
    if !cfg!(windows) {
        return Vec::new();
    }
    let output = match {
        let mut c = Command::new("wsl.exe");
        c.args(["-l", "-q"]);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            c.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }
        c.output()
    } {
        Ok(item) => item,
        Err(_) => return Vec::new(),
    };
    if !output.status.success() {
        return Vec::new();
    }
    let decoded = decode_wsl_output(&output.stdout).replace('\0', "");
    decoded
        .lines()
        .map(|item| item.trim().trim_start_matches('\u{feff}').to_string())
        .filter(|item| !item.is_empty())
        .collect()
}

fn save_openai_key_to_wsl(distro: &str, token: &str) -> Result<String> {
    let json_text = serde_json::to_string_pretty(&serde_json::json!({
        "OPENAI_API_KEY": token
    }))
    .with_context(|| "构建OpenAI配置内容失败")?;
    let script = format!(
        "mkdir -p \"$HOME/.codex\" && cat <<'EOF' > \"$HOME/.codex/auth.json\"\n{}\nEOF\nprintf %s \"$HOME/.codex/auth.json\"",
        json_text
    );
    let output = {
        let mut c = Command::new("wsl.exe");
        c.args(["-d", distro, "-e", "sh", "-lc", &script]);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            c.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }
        c.output()
            .with_context(|| format!("执行 wsl.exe 失败: {}", distro))?
    };
    if !output.status.success() {
        let stderr = decode_wsl_output(&output.stderr).replace('\0', "");
        anyhow::bail!("WSL[{}] 写入失败: {}", distro, stderr.trim());
    }
    let stdout = decode_wsl_output(&output.stdout).replace('\0', "");
    let path = stdout.trim();
    if path.is_empty() {
        Ok("$HOME/.codex/auth.json".to_string())
    } else {
        Ok(path.to_string())
    }
}

fn save_openai_key(token: &str) -> Result<String> {
    let local = save_openai_key_local(token)?;
    let mut summary = vec![format!("Windows: {}", local.display())];
    let distros = discover_wsl_distros();
    let mut wsl_success = 0_usize;
    let mut wsl_fail = 0_usize;
    for distro in distros {
        match save_openai_key_to_wsl(&distro, token) {
            Ok(path) => {
                wsl_success += 1;
                summary.push(format!("WSL[{distro}]: {path}"));
            }
            Err(err) => {
                wsl_fail += 1;
                summary.push(format!("WSL[{distro}]失败: {err}"));
            }
        }
    }
    summary.push(format!("WSL成功 {wsl_success}，失败 {wsl_fail}"));
    Ok(summary.join(" | "))
}
