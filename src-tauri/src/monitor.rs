use crate::api_client::{ApiBalanceClient, ApiBalanceResult};
use crate::models::{Account, AppConfig, CheckResult, ProgressEvent};
use crate::performance_monitor::{PerformanceMonitor, get_performance_monitor};
use crate::state::StateStore;
use crate::web_check::run_web_check;
use futures::stream::{FuturesUnordered, StreamExt};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;
use tauri::{AppHandle, Emitter};
use tokio::sync::{Mutex, Semaphore};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QueryMode {
    Normal,
    WebOnly,
}

/// 向前端发送实时进度日志
fn emit_progress(app: &AppHandle, level: &str, username: &str, message: &str) {
    let payload = ProgressEvent {
        level: level.to_string(),
        username: username.to_string(),
        message: message.to_string(),
    };
    if let Err(e) = app.emit("progress-log", &payload) {
        tracing::warn!("发送进度事件失败: {}", e);
    }
}

pub async fn check_accounts(
    accounts: Vec<Account>,
    config: AppConfig,
    state: Arc<Mutex<StateStore>>,
    target_username: Option<String>,
    app: AppHandle,
) -> Vec<CheckResult> {
    check_accounts_by_mode(accounts, config, state, target_username, QueryMode::Normal, app).await
}

pub async fn check_accounts_web_only(
    accounts: Vec<Account>,
    config: AppConfig,
    state: Arc<Mutex<StateStore>>,
    target_username: Option<String>,
    app: AppHandle,
) -> Vec<CheckResult> {
    check_accounts_by_mode(accounts, config, state, target_username, QueryMode::WebOnly, app).await
}

async fn check_accounts_by_mode(
    accounts: Vec<Account>,
    config: AppConfig,
    state: Arc<Mutex<StateStore>>,
    target_username: Option<String>,
    mode: QueryMode,
    app: AppHandle,
) -> Vec<CheckResult> {
    let perf_monitor = get_performance_monitor();
    let mut batch_meta = HashMap::new();
    batch_meta.insert(
        "target".to_string(),
        target_username.clone().unwrap_or_default(),
    );
    batch_meta.insert(
        "mode".to_string(),
        if mode == QueryMode::WebOnly {
            "web_only".to_string()
        } else {
            "normal".to_string()
        },
    );
    let batch_timer = PerformanceMonitor::start_operation(
        perf_monitor.clone(),
        if mode == QueryMode::WebOnly {
            "批量网页登录账号"
        } else {
            "批量查询账号"
        },
        batch_meta,
    );

    let started = Instant::now();
    let api_client = if mode == QueryMode::Normal {
        match ApiBalanceClient::new(&config.api.base_url, config.api.timeout) {
            Ok(item) => Some(Arc::new(item)),
            Err(e) => {
                let msg = format!("初始化API客户端失败: {e}");
                batch_timer.finish(false, Some(msg.clone()));
                emit_progress(&app, "error", "", &msg);
                return vec![CheckResult {
                    username: "SYSTEM".to_string(),
                    success: false,
                    balance_text: "错误".to_string(),
                    source: "init".to_string(),
                    message: msg,
                }];
            }
        }
    } else {
        None
    };

    let max_workers = config.performance.max_workers.max(1);
    let semaphore = Arc::new(Semaphore::new(max_workers));
    let selected: Vec<Account> = accounts
        .into_iter()
        .filter(|item| {
            target_username
                .as_ref()
                .map(|target| target == &item.username)
                .unwrap_or(true)
        })
        .collect();

    if mode == QueryMode::WebOnly {
        let msg = format!("开始仅网页登录检查 {} 个账号", selected.len());
        tracing::info!("{}", msg);
        emit_progress(&app, "info", "", &msg);
    } else {
        let msg = format!("开始检查 {} 个账号", selected.len());
        tracing::info!("{}", msg);
        emit_progress(&app, "info", "", &msg);
    }
    let mut jobs = FuturesUnordered::new();

    for account in selected.into_iter() {
        let semaphore = semaphore.clone();
        let api_client = api_client.clone();
        let config = config.clone();
        let state = state.clone();
        let perf_monitor = perf_monitor.clone();
        let app = app.clone();
        let perf_username = account.username.clone();
        jobs.push(tokio::spawn(async move {
            let mut account_meta = HashMap::new();
            account_meta.insert("username".to_string(), perf_username.clone());
            account_meta.insert(
                "mode".to_string(),
                if mode == QueryMode::WebOnly {
                    "web_only".to_string()
                } else {
                    "normal".to_string()
                },
            );
            let timer = PerformanceMonitor::start_operation(
                perf_monitor.clone(),
                if mode == QueryMode::WebOnly {
                    format!("网页登录账号_{perf_username}")
                } else {
                    format!("查询账号_{perf_username}")
                },
                account_meta,
            );

            let permit = semaphore
                .acquire_owned()
                .await
                .map_err(|e| format!("信号量获取失败: {e}"))?;
            let _guard = permit;

            let result = check_single_account(account, config, api_client, state, mode, &app).await;
            if result.success {
                timer.finish(true, None);
            } else {
                timer.finish(false, Some(result.message.clone()));
            }
            Ok::<CheckResult, String>(result)
        }));
    }

    let mut results = Vec::new();
    while let Some(item) = jobs.next().await {
        match item {
            Ok(Ok(result)) => results.push(result),
            Ok(Err(err)) => results.push(CheckResult {
                username: "SYSTEM".to_string(),
                success: false,
                balance_text: "错误".to_string(),
                source: "task".to_string(),
                message: err,
            }),
            Err(err) => results.push(CheckResult {
                username: "SYSTEM".to_string(),
                success: false,
                balance_text: "错误".to_string(),
                source: "task".to_string(),
                message: format!("任务Join失败: {err}"),
            }),
        }
    }
    results.sort_by(|a, b| a.username.cmp(&b.username));
    let success_count = results.iter().filter(|item| item.success).count();
    let fail_count = results.len().saturating_sub(success_count);
    let elapsed = started.elapsed().as_secs_f64();
    let summary = format!(
        "检查完成: 总数={}, 成功={}, 失败={}, 耗时={:.2}s",
        results.len(), success_count, fail_count, elapsed
    );
    tracing::info!("{}", summary);
    emit_progress(&app, "success", "", &summary);

    if success_count == results.len() {
        batch_timer.finish(true, None);
    } else {
        batch_timer.finish(
            false,
            Some(format!("本轮失败账号数={}", fail_count)),
        );
    }
    results
}

async fn check_single_account(
    account: Account,
    config: AppConfig,
    api_client: Option<Arc<ApiBalanceClient>>,
    state: Arc<Mutex<StateStore>>,
    mode: QueryMode,
    app: &AppHandle,
) -> CheckResult {
    if mode == QueryMode::WebOnly {
        return check_single_account_web_only(account, config, state, app).await;
    }

    let username = account.username.clone();
    tracing::info!("开始检查账号: {}", username);
    emit_progress(app, "info", &username, "开始检查");
    let Some(api_client) = api_client else {
        emit_progress(app, "error", &username, "API客户端未初始化");
        return CheckResult {
            username,
            success: false,
            balance_text: "错误".to_string(),
            source: "init".to_string(),
            message: "API客户端未初始化".to_string(),
        };
    };

    let force_web = {
        let guard = state.lock().await;
        guard.should_force_web_query(&username)
    };
    if force_web {
        tracing::info!("账号 {} 当前周期首次查询，需执行网页登录签到", username);
        emit_progress(app, "info", &username, "当前周期首次查询，需执行网页登录签到");
    }

    // 非强制网页时优先走API秒查
    if !force_web && !account.api_key.trim().is_empty() {
        emit_progress(app, "info", &username, "尝试API秒查...");
        let api_result = api_client.query_balance(&account.api_key).await;
        if api_result.success {
            return on_api_success(&username, api_result, state, app).await;
        }
        let msg = format!("API秒查失败: {}", api_result.message);
        tracing::warn!("账号 {} {}", username, msg);
        emit_progress(app, "warn", &username, &msg);
        if !config.api.fallback_to_web {
            return on_api_fail_without_web_fallback(&username, api_result, state, app).await;
        }
        emit_progress(app, "info", &username, "回退到网页登录...");
    }

    // 执行网页签到钩子
    emit_progress(app, "info", &username, "执行网页登录签到...");
    match run_web_check(
        &account,
        &config.web_check,
        &config.browser,
        config.performance.retry_times,
        config.performance.retry_delay,
    )
    .await
    {
        Ok(web_result) if web_result.success => {
            if let Some(balance) = web_result.balance {
                let balance_text = format_balance(balance);
                let mark_result = {
                    let mut guard = state.lock().await;
                    let mark = guard.mark_web_query_success(&username);
                    let save = guard.update_balance_cache(&username, &balance_text, None, None);
                    mark.and(save)
                };
                if let Err(e) = mark_result {
                    tracing::warn!("账号 {} 更新本地状态失败: {}", username, e);
                }

                // 网页成功后，同轮再尝试API秒刷新（成功则覆盖）
                if !account.api_key.trim().is_empty() {
                    emit_progress(app, "info", &username, "网页签到成功，尝试同轮API秒刷新...");
                    let post_api = api_client.query_balance(&account.api_key).await;
                    if post_api.success {
                        tracing::info!("账号 {} 同轮API秒刷新成功", username);
                        emit_progress(app, "success", &username, "同轮API秒刷新成功");
                        return on_api_success(&username, post_api, state, app).await;
                    }
                    let msg = format!("同轮API秒刷新失败，保留网页登录结果: {}", post_api.message);
                    tracing::warn!("账号 {} {}", username, msg);
                    emit_progress(app, "warn", &username, &msg);
                }

                let msg = format!("网页签到成功，余额 {}", balance_text);
                tracing::info!("账号 {} {}", username, msg);
                emit_progress(app, "success", &username, &msg);
                CheckResult {
                    username,
                    success: true,
                    balance_text,
                    source: "web_hook".to_string(),
                    message: "网页登录签到成功".to_string(),
                }
            } else {
                tracing::warn!("账号 {} 网页签到返回成功但没有余额字段", username);
                emit_progress(app, "warn", &username, "网页签到成功但未提取到余额");
                if force_web {
                    return CheckResult {
                        username,
                        success: false,
                        balance_text: "错误".to_string(),
                        source: "web_hook".to_string(),
                        message: "每日首查要求网页登录并成功提取余额，当前未提取到余额".to_string(),
                    };
                }
                // 没有余额值时，尝试API兜底返回
                if !account.api_key.trim().is_empty() {
                    emit_progress(app, "info", &username, "尝试API兜底查询余额...");
                    let api_result = api_client.query_balance(&account.api_key).await;
                    if api_result.success {
                        return on_api_success(&username, api_result, state, app).await;
                    }
                }
                CheckResult {
                    username,
                    success: false,
                    balance_text: "错误".to_string(),
                    source: "web_hook".to_string(),
                    message: "网页登录成功但未提取到余额".to_string(),
                }
            }
        }
        Ok(web_result) => {
            let msg = if force_web {
                format!("每日首查网页登录签到失败: {}", web_result.message)
            } else {
                format!("网页登录签到失败: {}", web_result.message)
            };
            emit_progress(app, "error", &username, &msg);
            CheckResult {
                username,
                success: false,
                balance_text: "错误".to_string(),
                source: "web_hook".to_string(),
                message: msg,
            }
        }
        Err(err) => {
            tracing::warn!("账号 {} 网页签到命令失败: {}", username, err);
            emit_progress(app, "error", &username, &format!("网页签到命令失败: {}", err));
            if force_web {
                return CheckResult {
                    username,
                    success: false,
                    balance_text: "错误".to_string(),
                    source: "web_hook".to_string(),
                    message: format!("每日首查网页登录不可用: {err}"),
                };
            }
            if !account.api_key.trim().is_empty() {
                emit_progress(app, "info", &username, "网页不可用，尝试API查询...");
                let api_result = api_client.query_balance(&account.api_key).await;
                if api_result.success {
                    return on_api_success(&username, api_result, state, app).await;
                }
            }
            CheckResult {
                username,
                success: false,
                balance_text: "错误".to_string(),
                source: "web_hook".to_string(),
                message: format!("网页登录不可用: {err}"),
            }
        }
    }
}

async fn check_single_account_web_only(
    account: Account,
    config: AppConfig,
    state: Arc<Mutex<StateStore>>,
    app: &AppHandle,
) -> CheckResult {
    let username = account.username.clone();
    tracing::info!("开始仅网页登录账号: {}", username);
    emit_progress(app, "info", &username, "开始仅网页登录");

    match run_web_check(
        &account,
        &config.web_check,
        &config.browser,
        config.performance.retry_times,
        config.performance.retry_delay,
    )
    .await
    {
        Ok(web_result) if web_result.success => {
            if let Some(balance) = web_result.balance {
                let balance_text = format_balance(balance);
                let mark_result = {
                    let mut guard = state.lock().await;
                    let mark = guard.mark_web_query_success(&username);
                    let save = guard.update_balance_cache(&username, &balance_text, None, None);
                    mark.and(save)
                };
                if let Err(e) = mark_result {
                    tracing::warn!("账号 {} 更新本地状态失败: {}", username, e);
                }

                let msg = format!("仅网页登录成功，余额 {}", balance_text);
                tracing::info!("账号 {} {}", username, msg);
                emit_progress(app, "success", &username, &msg);
                CheckResult {
                    username,
                    success: true,
                    balance_text,
                    source: "web_only".to_string(),
                    message: if web_result.message.trim().is_empty() {
                        "仅网页登录成功".to_string()
                    } else {
                        web_result.message
                    },
                }
            } else {
                emit_progress(app, "warn", &username, "网页登录成功但未提取到余额");
                CheckResult {
                    username,
                    success: false,
                    balance_text: "错误".to_string(),
                    source: "web_only".to_string(),
                    message: "网页登录成功但未提取到余额".to_string(),
                }
            }
        }
        Ok(web_result) => {
            let msg = format!("网页登录失败: {}", web_result.message);
            emit_progress(app, "error", &username, &msg);
            CheckResult {
                username,
                success: false,
                balance_text: "错误".to_string(),
                source: "web_only".to_string(),
                message: msg,
            }
        }
        Err(err) => {
            let msg = format!("网页登录不可用: {err}");
            emit_progress(app, "error", &username, &msg);
            CheckResult {
                username,
                success: false,
                balance_text: "错误".to_string(),
                source: "web_only".to_string(),
                message: msg,
            }
        }
    }
}

async fn on_api_success(
    username: &str,
    api_result: ApiBalanceResult,
    state: Arc<Mutex<StateStore>>,
    app: &AppHandle,
) -> CheckResult {
    let balance = api_result.balance.unwrap_or_default();
    let balance_text = format_balance(balance);
    {
        let mut guard = state.lock().await;
        if let Err(e) = guard.update_balance_cache(username, &balance_text, None, None) {
            tracing::warn!("账号 {} 保存余额缓存失败: {}", username, e);
        }
    }
    let msg = format!("API秒查成功: {} (source={})", balance_text, api_result.source);
    tracing::info!("账号 {} {}", username, msg);
    emit_progress(app, "success", username, &msg);
    CheckResult {
        username: username.to_string(),
        success: true,
        balance_text,
        source: api_result.source,
        message: api_result.message,
    }
}

async fn on_api_fail_without_web_fallback(
    username: &str,
    api_result: ApiBalanceResult,
    state: Arc<Mutex<StateStore>>,
    app: &AppHandle,
) -> CheckResult {
    let cached = {
        let guard = state.lock().await;
        guard.get_cached_balance_text(username)
    };
    if let Some(balance_text) = cached {
        let msg = format!("API秒查失败，回退缓存结果: {}", api_result.message);
        tracing::warn!("账号 {} {}", username, msg);
        emit_progress(app, "warn", username, &msg);
        return CheckResult {
            username: username.to_string(),
            success: true,
            balance_text,
            source: "cache".to_string(),
            message: format!("API失败，使用缓存: {}", api_result.message),
        };
    }

    emit_progress(app, "error", username, &format!("API查询失败: {}", api_result.message));
    CheckResult {
        username: username.to_string(),
        success: false,
        balance_text: "API失败".to_string(),
        source: "api".to_string(),
        message: api_result.message,
    }
}

fn format_balance(value: f64) -> String {
    format!("${value:.1}")
}
