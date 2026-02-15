use crate::models::BrowserConfig;
use crate::models::{Account, WebCheckConfig};
use crate::utils::parse_first_number;
use crate::web_native::run_native_web_check;
use anyhow::{Context, Result};
use serde::Deserialize;
use tokio::process::Command;
use tokio::time::{Duration, timeout};

#[derive(Debug, Clone)]
pub struct WebCheckResult {
    pub success: bool,
    pub balance: Option<f64>,
    pub message: String,
}

#[derive(Debug, Deserialize)]
struct HookJsonResult {
    #[serde(default)]
    success: bool,
    #[serde(default)]
    balance: Option<f64>,
    #[serde(default)]
    message: String,
}

pub async fn run_web_check(
    account: &Account,
    config: &WebCheckConfig,
    browser_config: &BrowserConfig,
    retry_times: u32,
    retry_delay_secs: u64,
) -> Result<WebCheckResult> {
    if !config.enabled {
        tracing::debug!("web_check.enabled=false，回退到原生网页登录流程");
        return run_native_web_check(
            account,
            config,
            browser_config,
            retry_times,
            retry_delay_secs,
        )
        .await;
    }

    // command 为空时走原生Rust网页登录流程
    if config.command.trim().is_empty() {
        return run_native_web_check(
            account,
            config,
            browser_config,
            retry_times,
            retry_delay_secs,
        )
        .await;
    }

    let mut command = Command::new(config.command.trim());
    for raw_arg in &config.args {
        let arg = raw_arg
            .replace("{username}", &account.username)
            .replace("{password}", &account.password)
            .replace("{api_key}", &account.api_key);
        command.arg(arg);
    }
    #[cfg(windows)]
    {
        command.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }

    let timeout_secs = config.timeout_seconds.max(5);
    let output = timeout(Duration::from_secs(timeout_secs), command.output())
        .await
        .with_context(|| format!("网页签到命令执行超时({timeout_secs}s)"))?
        .with_context(|| "网页签到命令执行失败")?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

    if !output.status.success() {
        anyhow::bail!(
            "网页签到命令返回非0: code={:?}, stderr={}",
            output.status.code(),
            stderr
        );
    }

    if stdout.is_empty() {
        return Ok(WebCheckResult {
            success: true,
            balance: None,
            message: "网页签到命令执行成功，但未返回余额".to_string(),
        });
    }

    if let Ok(json_result) = serde_json::from_str::<HookJsonResult>(&stdout) {
        return Ok(WebCheckResult {
            success: json_result.success,
            balance: json_result.balance,
            message: if json_result.message.is_empty() {
                "网页签到命令返回JSON".to_string()
            } else {
                json_result.message
            },
        });
    }

    let balance = parse_first_number(&stdout);
    Ok(WebCheckResult {
        success: true,
        balance,
        message: "网页签到命令返回文本".to_string(),
    })
}
