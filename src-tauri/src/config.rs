use crate::models::{Account, AppConfig};
use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct RuntimeFiles {
    pub config_dir: PathBuf,
    pub config_file: PathBuf,
    pub credentials_file: PathBuf,
    pub balance_cache_file: PathBuf,
    pub daily_web_state_file: PathBuf,
}

impl RuntimeFiles {
    pub fn new(config_dir: PathBuf) -> Self {
        Self {
            config_file: config_dir.join("config.json"),
            credentials_file: config_dir.join("credentials.txt"),
            balance_cache_file: config_dir.join("balance_cache.json"),
            daily_web_state_file: config_dir.join("daily_web_login_state.json"),
            config_dir,
        }
    }
}

pub fn load_app_config(config_file: &Path) -> Result<AppConfig> {
    if !config_file.exists() {
        return Ok(AppConfig::default());
    }
    let raw = fs::read_to_string(config_file)
        .with_context(|| format!("读取配置文件失败: {}", config_file.display()))?;
    let config: AppConfig = serde_json::from_str(&raw).with_context(|| "解析 config.json 失败")?;
    Ok(config)
}

pub fn load_accounts(credentials_file: &Path) -> Result<Vec<Account>> {
    if !credentials_file.exists() {
        return Ok(Vec::new());
    }
    let raw = fs::read_to_string(credentials_file)
        .with_context(|| format!("读取账号文件失败: {}", credentials_file.display()))?;
    let mut accounts = Vec::new();
    for (idx, line) in raw.lines().enumerate() {
        let text = line.trim();
        if text.is_empty() || text.starts_with('#') {
            continue;
        }

        let mut parts = text.splitn(3, ',').map(|item| item.trim());
        let username = parts.next().unwrap_or_default();
        let password = parts.next().unwrap_or_default();
        let api_key = parts.next().unwrap_or_default();

        if username.is_empty() || password.is_empty() {
            tracing::warn!("账号文件第{}行格式无效，已跳过", idx + 1);
            continue;
        }

        accounts.push(Account {
            username: username.to_string(),
            password: password.to_string(),
            api_key: api_key.to_string(),
        });
    }
    Ok(accounts)
}

pub fn save_accounts(credentials_file: &Path, accounts: &[Account]) -> Result<()> {
    let mut lines = Vec::new();
    lines.push("# AnyRouter账号配置文件".to_string());
    lines.push("# 格式: 用户名,密码,API_KEY(可选)".to_string());
    for account in accounts {
        let mut line = format!("{},{}", account.username, account.password);
        if !account.api_key.trim().is_empty() {
            line.push(',');
            line.push_str(account.api_key.trim());
        }
        lines.push(line);
    }
    let content = lines.join("\n") + "\n";
    fs::write(credentials_file, content)
        .with_context(|| format!("写入账号文件失败: {}", credentials_file.display()))?;
    Ok(())
}

pub fn remove_account(credentials_file: &Path, username: &str) -> Result<bool> {
    let mut accounts = load_accounts(credentials_file)?;
    let before = accounts.len();
    accounts.retain(|item| item.username != username);
    if accounts.len() == before {
        return Ok(false);
    }
    save_accounts(credentials_file, &accounts)?;
    Ok(true)
}
