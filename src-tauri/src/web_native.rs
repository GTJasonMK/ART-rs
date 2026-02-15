use crate::browser_pool::get_global_pool;
use crate::models::{Account, BrowserConfig, WebCheckConfig};
use crate::utils::{parse_first_number, value_to_f64 as to_f64};
use crate::web_check::WebCheckResult;
use anyhow::{Context, Result};
use serde_json::Value;
use std::time::{Duration, Instant};
use thirtyfour::common::capabilities::chromium::ChromiumLikeCapabilities;
use thirtyfour::prelude::*;
use tokio::task;
use tokio::time::sleep as async_sleep;

const CONSOLE_URL: &str = "https://anyrouter.top/console";
const QUOTA_UNIT_PER_DOLLAR: f64 = 500000.0;

pub async fn run_native_web_check(
    account: &Account,
    web_config: &WebCheckConfig,
    browser_config: &BrowserConfig,
    retry_times: u32,
    retry_delay_secs: u64,
) -> Result<WebCheckResult> {
    let web_cfg = web_config.clone();
    let pool = task::spawn_blocking(move || get_global_pool(&web_cfg))
        .await
        .map_err(|e| anyhow::anyhow!("初始化浏览器池任务失败: {e}"))?
        .with_context(|| "初始化浏览器池失败")?;
    let ticket = {
        let acquire_timeout = Duration::from_secs(20);
        let started = Instant::now();
        loop {
            {
                let mut guard = pool
                    .lock()
                    .map_err(|_| anyhow::anyhow!("浏览器池锁获取失败"))?;
                match guard
                    .try_acquire()
                    .with_context(|| "从浏览器池获取可用实例失败")?
                {
                    Some(ticket) => break ticket,
                    None => {} // 当前无可用实例，释放锁后等待重试
                }
            } // guard 在此处 drop，释放锁
            if started.elapsed() >= acquire_timeout {
                anyhow::bail!("等待浏览器池可用实例超时({}s)", acquire_timeout.as_secs());
            }
            async_sleep(Duration::from_millis(120)).await;
        }
    };

    let mut caps = DesiredCapabilities::chrome();
    caps.add_arg("--disable-gpu")?;
    caps.add_arg("--no-sandbox")?;
    caps.add_arg("--disable-dev-shm-usage")?;
    caps.add_arg("--disable-extensions")?;
    caps.add_arg("--disable-background-networking")?;
    caps.add_arg("--disable-component-update")?;
    caps.add_arg("--disable-default-apps")?;
    caps.add_arg("--disable-sync")?;
    caps.add_arg("--no-first-run")?;
    caps.add_arg("--no-default-browser-check")?;
    caps.add_arg("--disable-features=DirectComposition,CalculateNativeWinOcclusion,MediaRouter")?;
    caps.add_arg("--log-level=3")?;
    caps.add_arg("--disable-logging")?;
    caps.add_experimental_option("excludeSwitches", serde_json::json!(["enable-logging"]))?;
    if browser_config.headless {
        caps.add_arg("--headless=new")?;
    }
    if browser_config.disable_javascript {
        caps.add_arg("--disable-javascript")?;
    }
    let window_arg = format!("--window-size={}", browser_config.window_size);
    caps.add_arg(&window_arg)?;
    if let Some(ua) = &browser_config.user_agent {
        if !ua.trim().is_empty() {
            let user_agent_arg = format!("--user-agent={}", ua.trim());
            caps.add_arg(&user_agent_arg)?;
        }
    }
    if browser_config.disable_images {
        caps.add_experimental_option(
            "prefs",
            serde_json::json!({"profile.managed_default_content_settings.images": 2}),
        )?;
    }

    let driver = WebDriver::new(&ticket.url, caps)
        .await
        .with_context(|| "连接 chromedriver 失败")?;

    let timeout_secs = web_config.timeout_seconds.max(20);
    let result = tokio::time::timeout(
        Duration::from_secs(timeout_secs),
        run_login_flow_with_retry(
            &driver,
            account,
            browser_config,
            retry_times,
            retry_delay_secs,
        ),
    )
    .await;

    let final_result = match result {
        Ok(Ok(data)) => data,
        Ok(Err(err)) => WebCheckResult {
            success: false,
            balance: None,
            message: format!("网页流程失败: {err}"),
        },
        Err(_) => WebCheckResult {
            success: false,
            balance: None,
            message: format!("网页流程超时({timeout_secs}s)"),
        },
    };

    let _ = driver.quit().await;
    if let Ok(mut guard) = pool.lock() {
        guard.release(ticket);
        let stats = guard.get_stats();
        tracing::debug!(
            "浏览器池统计: 复用率={:.1}%, 可用实例={}",
            stats.get("reuse_rate").copied().unwrap_or(0.0),
            stats.get("available_count").copied().unwrap_or(0.0)
        );
    }
    Ok(final_result)
}

async fn run_login_flow_with_retry(
    driver: &WebDriver,
    account: &Account,
    browser_config: &BrowserConfig,
    retry_times: u32,
    retry_delay_secs: u64,
) -> Result<WebCheckResult> {
    let retry_times = retry_times.max(1);
    let retry_delay_secs = retry_delay_secs.max(1);
    let mut last_error = String::new();
    for attempt in 0..retry_times {
        match run_login_flow_once(driver, account, browser_config).await {
            Ok(result) => return Ok(result),
            Err(err) => {
                last_error = err.to_string();
                tracing::warn!(
                    "登录失败 (尝试 {}/{}): {}",
                    attempt + 1,
                    retry_times,
                    last_error
                );
                if attempt + 1 < retry_times {
                    async_sleep(Duration::from_secs(retry_delay_secs)).await;
                }
            }
        }
    }
    anyhow::bail!("登录失败，已重试{}次: {}", retry_times, last_error)
}

async fn run_login_flow_once(
    driver: &WebDriver,
    account: &Account,
    browser_config: &BrowserConfig,
) -> Result<WebCheckResult> {
    let flow_started = Instant::now();

    driver.get(CONSOLE_URL).await.with_context(|| "导航到控制台失败")?;
    async_sleep(Duration::from_millis(800)).await;

    let current_url = driver.current_url().await?.to_string();
    if current_url.contains("/login") {
        let step_started = Instant::now();
        async_sleep(Duration::from_millis(500)).await;
        close_announcement_popup(driver).await?;
        switch_to_email_login(driver).await?;
        submit_login(driver, account).await?;
        driver.get(CONSOLE_URL).await.with_context(|| "登录后导航到控制台失败")?;
        async_sleep(Duration::from_millis(800)).await;
        tracing::debug!("[flow] 登录流程耗时={:.1}s", step_started.elapsed().as_secs_f64());
    }

    let logged_url = driver.current_url().await?.to_string();
    tracing::info!("[flow] 登录后URL: {}", logged_url);
    if !logged_url.contains("/console") || logged_url.contains("/login") {
        if let Some(error_text) = check_login_error_message(driver).await {
            anyhow::bail!("登录失败: {} (当前URL: {})", error_text, logged_url);
        }
        anyhow::bail!("登录失败，当前URL: {logged_url}");
    }

    let step_started = Instant::now();
    let balance = extract_balance(driver, browser_config.timeout.max(3))
        .await
        .with_context(|| "余额提取失败")?;
    let balance_num =
        parse_first_number(&balance).with_context(|| format!("余额格式无法解析: {balance}"))?;
    tracing::debug!("[flow] 余额提取耗时={:.1}s, balance={}", step_started.elapsed().as_secs_f64(), balance);

    let step_started = Instant::now();
    let sync_msg = match sync_first_apikey_limit(driver, balance_num).await {
        Ok(msg) => msg,
        Err(err) => {
            tracing::warn!("同步首个 API Key 额度失败: {}", err);
            format!("同步额度失败: {err}")
        }
    };
    tracing::debug!("[flow] sync_first_apikey_limit 耗时={:.1}s", step_started.elapsed().as_secs_f64());
    tracing::debug!("[flow] run_login_flow_once 总耗时={:.1}s", flow_started.elapsed().as_secs_f64());

    Ok(WebCheckResult {
        success: true,
        balance: Some(balance_num),
        message: sync_msg,
    })
}

async fn check_login_error_message(driver: &WebDriver) -> Option<String> {
    let script = r#"
        const selectors = ['.error-message', '.alert-danger', '.toast-error', '[role="alert"]'];
        for (const selector of selectors) {
            const node = document.querySelector(selector);
            if (!node) continue;
            const text = (node.innerText || node.textContent || '').trim();
            if (text) return text;
        }
        return '';
    "#;
    let value = driver.execute(script, Vec::<Value>::new()).await.ok()?;
    let text = value.json().as_str().unwrap_or("").trim().to_string();
    if text.is_empty() { None } else { Some(text) }
}

async fn close_announcement_popup(driver: &WebDriver) -> Result<()> {
    let script = r#"
        const closeBtn = document.querySelector('.semi-modal-close');
        if (closeBtn && closeBtn.offsetParent !== null) {
            closeBtn.click();
            return true;
        }
        const buttons = Array.from(document.querySelectorAll('button'));
        for (const btn of buttons) {
            const text = (btn.textContent || '').trim();
            if (text.includes('今日关闭') || text.includes('关闭公告') || text.includes('关闭')) {
                btn.click();
                return true;
            }
        }
        return false;
    "#;
    let _ = driver.execute(script, Vec::<Value>::new()).await?;
    Ok(())
}

async fn switch_to_email_login(driver: &WebDriver) -> Result<()> {
    if let Ok(btn) = driver
        .find(By::Css("button[type='button'] span.semi-icon-mail"))
        .await
    {
        let _ = driver
            .execute("arguments[0].click();", vec![btn.to_json()?])
            .await;
        async_sleep(Duration::from_millis(1500)).await;
    }
    Ok(())
}

async fn submit_login(driver: &WebDriver, account: &Account) -> Result<()> {
    let username = driver
        .query(By::Name("username"))
        .wait(Duration::from_secs(5), Duration::from_millis(200))
        .first()
        .await
        .with_context(|| "未找到用户名输入框")?;
    let password = driver
        .query(By::Name("password"))
        .wait(Duration::from_secs(5), Duration::from_millis(200))
        .first()
        .await
        .with_context(|| "未找到密码输入框")?;

    username.clear().await?;
    username.send_keys(&account.username).await?;
    password.clear().await?;
    password.send_keys(&account.password).await?;

    let submit_btn = driver
        .find(By::Css("button[type='submit']"))
        .await
        .with_context(|| "未找到提交按钮")?;
    driver
        .execute("arguments[0].click();", vec![submit_btn.to_json()?])
        .await
        .with_context(|| "点击提交按钮失败")?;

    async_sleep(Duration::from_millis(2000)).await;
    Ok(())
}

async fn extract_balance(driver: &WebDriver, wait_time: u64) -> Result<String> {
    // 等待骨架屏消失(参考Python版BalanceExtractor，确保数据已渲染)
    let skeleton_script = r#"
        return !document.querySelector('.semi-skeleton');
    "#;
    let skeleton_started = Instant::now();
    while skeleton_started.elapsed() < Duration::from_secs(10) {
        match driver.execute(skeleton_script, Vec::<Value>::new()).await {
            Ok(result) if result.json().as_bool().unwrap_or(false) => break,
            _ => {}
        }
        async_sleep(Duration::from_millis(300)).await;
    }
    // 骨架屏消失后再等1s确保数据完全渲染
    async_sleep(Duration::from_millis(1000)).await;

    let extract_script = r#"
        function extractBalance() {
            const knownSelectors = [
                '.balance-amount',
                '[data-balance]',
                '.amount-display',
                '.wallet-balance',
                '.user-balance',
                '.account-balance',
                '.current-balance',
                'span[class*="balance"]',
                'div[class*="balance"]'
            ];
            for (const selector of knownSelectors) {
                try {
                    const elems = document.querySelectorAll(selector);
                    for (const elem of elems) {
                        const text = String(elem.textContent || '');
                        if (!text.includes('$')) continue;
                        const match = text.match(/\$([\d,]+\.?\d*)/);
                        if (match) {
                            const value = parseFloat(String(match[1] || '').replace(/,/g, ''));
                            if (Number.isFinite(value) && value > 0) {
                                return '$' + value.toFixed(1);
                            }
                        }
                    }
                } catch (e) {}
            }

            const balanceTexts = ['当前余额', 'Current Balance', '余额', 'Balance'];
            for (const key of balanceTexts) {
                try {
                    const xpath = `//*[contains(text(), '${key}')]`;
                    const result = document.evaluate(
                        xpath,
                        document,
                        null,
                        XPathResult.FIRST_ORDERED_NODE_TYPE,
                        null
                    );
                    const node = result.singleNodeValue;
                    if (!node) continue;

                    const parent = node.parentElement;
                    if (parent) {
                        const siblings = Array.from(parent.children);
                        for (const item of siblings) {
                            const m = String(item.textContent || '').match(/\$([\d,]+\.?\d*)/);
                            if (m) {
                                const value = parseFloat(String(m[1] || '').replace(/,/g, ''));
                                if (Number.isFinite(value) && value > 0) {
                                    return '$' + value.toFixed(1);
                                }
                            }
                        }
                        const p = String(parent.textContent || '').match(/\$([\d,]+\.?\d*)/);
                        if (p) {
                            const value = parseFloat(String(p[1] || '').replace(/,/g, ''));
                            if (Number.isFinite(value) && value > 0) {
                                return '$' + value.toFixed(1);
                            }
                        }
                    }
                } catch (e) {}
            }

            const largeTextSelectors = [
                '.text-lg', '.text-xl', '.text-2xl', '.text-3xl',
                'h1', 'h2', 'h3',
                '[style*="font-size: 2"]', '[style*="font-size: 3"]'
            ];
            for (const selector of largeTextSelectors) {
                const elems = document.querySelectorAll(selector);
                for (const elem of elems) {
                    const text = String(elem.textContent || '').trim();
                    if (!/^\$\s*[\d,]+\.?\d*$/.test(text)) continue;
                    const value = parseFloat(text.replace(/[$,\s]/g, ''));
                    if (Number.isFinite(value) && value > 0) {
                        return '$' + value.toFixed(1);
                    }
                }
            }

            const containerSelectors = [
                '.dashboard', '.console', '.account-info',
                '.user-panel', '.wallet', 'main', '#app'
            ];
            for (const containerSel of containerSelectors) {
                const container = document.querySelector(containerSel);
                if (!container) continue;
                const nodes = container.querySelectorAll('span, div, p');
                for (const node of nodes) {
                    const text = String(node.textContent || '').trim();
                    if (node.childElementCount !== 0) continue;
                    if (!/^\$\s*[\d,]+\.?\d*$/.test(text)) continue;
                    const value = parseFloat(text.replace(/[$,\s]/g, ''));
                    if (Number.isFinite(value) && value > 0) {
                        return '$' + value.toFixed(1);
                    }
                }
            }

            const bodyText = (document.body && document.body.innerText) ? document.body.innerText : '';
            const patterns = [
                /当前余额[：:\s]*\$([\d,]+\.?\d*)/,
                /余额[：:\s]*\$([\d,]+\.?\d*)/,
                /Balance[：:\s]*\$([\d,]+\.?\d*)/i
            ];
            for (const pattern of patterns) {
                const match = bodyText.match(pattern);
                if (match) {
                    const value = parseFloat(String(match[1] || '').replace(/,/g, ''));
                    if (Number.isFinite(value) && value > 0) {
                        return '$' + value.toFixed(1);
                    }
                }
            }
            return '';
        }

        return extractBalance();
    "#;

    // 轮询式提取: 每500ms尝试一次，最多等待 wait_time 秒
    let timeout = Duration::from_secs(wait_time);
    let started = Instant::now();
    loop {
        let result = driver.execute(extract_script, Vec::<Value>::new()).await?;
        let text = result.json().as_str().unwrap_or("").trim().to_string();
        if !text.is_empty() {
            return Ok(text);
        }
        if started.elapsed() >= timeout {
            break;
        }
        async_sleep(Duration::from_millis(500)).await;
    }

    // 诊断: 提取失败时抓取页面关键文本
    let diag_script = r#"
        const url = window.location.href || '';
        const body = (document.body && document.body.innerText) || '';
        const snippet = body.substring(0, 500).replace(/\s+/g, ' ');
        return { url, snippet };
    "#;
    if let Ok(diag) = driver.execute(diag_script, Vec::<Value>::new()).await {
        let obj = diag.json();
        let url = obj.get("url").and_then(|v| v.as_str()).unwrap_or("");
        let snippet = obj.get("snippet").and_then(|v| v.as_str()).unwrap_or("");
        tracing::warn!("[extract_balance] 超时未提取到余额, url={}, 页面前500字: {}", url, snippet);
    }

    anyhow::bail!("未提取到余额文本")
}

async fn sync_first_apikey_limit(driver: &WebDriver, balance: f64) -> Result<String> {
    let total_started = Instant::now();

    let step_started = Instant::now();
    open_apikey_page(driver).await?;
    tracing::debug!("[sync_quota] open_apikey_page 耗时={:.1}s", step_started.elapsed().as_secs_f64());

    let step_started = Instant::now();
    open_first_token_editor(driver).await?;
    tracing::debug!("[sync_quota] open_first_token_editor 耗时={:.1}s", step_started.elapsed().as_secs_f64());

    let step_started = Instant::now();
    let unit_rate = detect_quota_unit_rate(driver)
        .await
        .unwrap_or(QUOTA_UNIT_PER_DOLLAR);
    tracing::debug!("[sync_quota] detect_quota_unit_rate 耗时={:.1}s, rate={}", step_started.elapsed().as_secs_f64(), unit_rate);

    let target_quota = (balance * unit_rate).round().max(0.0) as i64;

    let step_started = Instant::now();
    set_modal_quota_value(driver, target_quota).await?;
    tracing::debug!("[sync_quota] set_modal_quota_value 耗时={:.1}s", step_started.elapsed().as_secs_f64());

    let step_started = Instant::now();
    submit_quota_modal(driver).await?;
    tracing::debug!("[sync_quota] submit_quota_modal 耗时={:.1}s", step_started.elapsed().as_secs_f64());

    tracing::debug!("[sync_quota] 总耗时={:.1}s", total_started.elapsed().as_secs_f64());

    Ok(format!(
        "首个 API Key 额度已同步: 余额=${:.2}, 额度值={}, 比例={:.2}",
        balance, target_quota, unit_rate
    ))
}

async fn open_apikey_page(driver: &WebDriver) -> Result<()> {
    let click_menu_script = r#"
        const xpath = "//*[self::a or self::button or self::span or self::div][normalize-space(text())='API令牌']";
        const node = document.evaluate(
            xpath,
            document,
            null,
            XPathResult.FIRST_ORDERED_NODE_TYPE,
            null
        ).singleNodeValue;
        if (!node) {
            return { ok: false, reason: 'menu_not_found' };
        }
        let clickable = node;
        while (clickable) {
            const tag = (clickable.tagName || '').toLowerCase();
            const role = clickable.getAttribute ? (clickable.getAttribute('role') || '').toLowerCase() : '';
            const cls = clickable.className ? String(clickable.className).toLowerCase() : '';
            if (
                tag === 'a' ||
                tag === 'button' ||
                role === 'button' ||
                cls.includes('semi-navigation-item')
            ) {
                break;
            }
            clickable = clickable.parentElement;
        }
        clickable = clickable || node;
        clickable.click();
        return { ok: true };
    "#;

    let clicked = driver
        .execute(click_menu_script, Vec::<Value>::new())
        .await?;
    let clicked_obj = clicked.json();
    if !clicked_obj
        .get("ok")
        .and_then(Value::as_bool)
        .unwrap_or(false)
    {
        tracing::debug!("未找到左侧 API令牌 菜单，回退直达 token 页面");
        driver.get("https://anyrouter.top/console/token").await?;
        async_sleep(Duration::from_millis(1200)).await;
    }

    let wait_loaded_script = r#"
        const text = document.body && document.body.innerText ? document.body.innerText : '';
        const onTokenPage = (window.location && window.location.href || '').includes('/console/token');
        return text.includes('添加令牌') || text.includes('复制所选令牌到剪贴板') || onTokenPage;
    "#;
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(8) {
        let loaded = driver
            .execute(wait_loaded_script, Vec::<Value>::new())
            .await?;
        if loaded.json().as_bool().unwrap_or(false) {
            return Ok(());
        }
        async_sleep(Duration::from_millis(200)).await;
    }
    anyhow::bail!("API令牌 页面未加载完成")
}

async fn open_first_token_editor(driver: &WebDriver) -> Result<()> {
    // 检测编辑弹窗是否已打开的脚本
    let editor_open_script = r#"
        function isVisible(node) {
            if (!node) return false;
            const style = window.getComputedStyle(node);
            if (style.display === 'none' || style.visibility === 'hidden') return false;
            const rect = node.getBoundingClientRect();
            return rect.width > 0 && rect.height > 0;
        }
        const hasEditorHeader = Array.from(document.querySelectorAll('*')).some((node) => {
            if (!isVisible(node)) return false;
            const text = (node.textContent || '').trim();
            return text.includes('更新令牌信息') || text.includes('额度设置') || text.includes('编辑令牌');
        });

        const hasQuotaLabel = Array.from(document.querySelectorAll('*')).some((node) => {
            if (!isVisible(node)) return false;
            return (node.textContent || '').trim() === '额度';
        });

        const hasSubmit = Array.from(document.querySelectorAll('button, [role="button"]')).some((btn) => {
            if (!isVisible(btn)) return false;
            if (btn.disabled) return false;
            const text = (btn.innerText || btn.textContent || '').trim();
            return text.includes('提交');
        });

        return hasEditorHeader || (hasQuotaLabel && hasSubmit);
    "#;

    // 等待令牌行出现的脚本
    let wait_row_script = r#"
        function isVisible(node) {
            if (!node) return false;
            const style = window.getComputedStyle(node);
            if (style.display === 'none' || style.visibility === 'hidden') return false;
            const rect = node.getBoundingClientRect();
            return rect.width > 0 && rect.height > 0;
        }
        function normalizeText(text) {
            return String(text || '').replace(/\s+/g, ' ').trim();
        }
        function isCopyLike(text) {
            const lower = normalizeText(text).toLowerCase();
            return lower === '复制' || lower.includes('复制') || lower.includes('拷贝') || lower === 'copy' || lower.includes('copy');
        }
        function isEditLike(text) {
            const lower = normalizeText(text).toLowerCase();
            return lower === '编辑' || lower.includes('编辑') || lower.includes('修改') || lower === 'edit' || lower.includes('edit');
        }
        function isNoDataText(text) {
            const lower = normalizeText(text).toLowerCase();
            return lower.includes('暂无数据') || lower.includes('no data') || lower.includes('no records') || lower.includes('empty');
        }
        function hasTokenActions(row) {
            if (!row) return false;
            const texts = Array.from(row.querySelectorAll('button, a, [role="button"], span, div, i'))
                .map((node) => normalizeText(node.innerText || node.textContent || ''))
                .filter((text) => !!text);
            const hasCopy = texts.some((text) => isCopyLike(text));
            const hasEdit = texts.some((text) => isEditLike(text));
            return hasCopy && hasEdit;
        }
        function hasActionControls(row) {
            if (!row) return false;
            const controls = Array.from(row.querySelectorAll('button, a, [role="button"], span, div, i')).filter((node) => {
                if (!isVisible(node)) return false;
                const text = normalizeText(node.innerText || node.textContent || '');
                const cls = String(node.className || '').toLowerCase();
                const role = String(node.getAttribute ? (node.getAttribute('role') || '') : '').toLowerCase();
                return (
                    !!text ||
                    cls.includes('icon') ||
                    cls.includes('more') ||
                    cls.includes('action') ||
                    role === 'button'
                );
            });
            return controls.length >= 2;
        }
        function isLikelyTokenRow(row) {
            const text = normalizeText(row ? row.innerText : '');
            if (!text) return false;
            if (row && row.querySelector('th')) return false;
            if (isNoDataText(text)) return false;
            if (hasTokenActions(row)) return true;
            const lower = text.toLowerCase();
            const columnCount = row ? row.querySelectorAll('td').length : 0;
            if (hasActionControls(row) && columnCount >= 4) return true;
            return (
                (text.includes('已启用') || lower.includes('enabled')) &&
                (text.includes('用户分组') || lower.includes('group')) &&
                isEditLike(text)
            );
        }
        const allRows = Array.from(
            document.querySelectorAll('tbody tr, .semi-table-tbody .semi-table-row, .semi-table-row')
        ).filter((row) => isVisible(row) && !(row && row.querySelector('th')));
        const rows = allRows.filter((row) => isLikelyTokenRow(row));
        const hasEmptyState = isNoDataText((document.body && document.body.innerText) || '');
        return {
            hasTokenRow: rows.length > 0,
            hasEmptyState
        };
    "#;

    // 首行直点编辑脚本(实测100%成功的唯一策略)
    let direct_click_script = r#"
        function isVisible(node) {
            if (!node) return false;
            const style = window.getComputedStyle(node);
            if (style.display === 'none' || style.visibility === 'hidden') return false;
            const rect = node.getBoundingClientRect();
            return rect.width > 0 && rect.height > 0;
        }
        function normalizeText(text) {
            return String(text || '').replace(/\s+/g, ' ').trim();
        }
        function toClickable(node) {
            let cursor = node;
            while (cursor) {
                const tag = (cursor.tagName || '').toLowerCase();
                const role = cursor.getAttribute ? (cursor.getAttribute('role') || '').toLowerCase() : '';
                if (tag === 'button' || tag === 'a' || role === 'button') {
                    return cursor;
                }
                cursor = cursor.parentElement;
            }
            return node;
        }
        function isEnabled(node) {
            if (!node) return false;
            if (node.disabled) return false;
            const aria = node.getAttribute ? (node.getAttribute('aria-disabled') || '').toLowerCase() : '';
            if (aria === 'true') return false;
            const style = window.getComputedStyle(node);
            if (style.pointerEvents === 'none') return false;
            return true;
        }
        function isCopyLike(text) {
            const lower = normalizeText(text).toLowerCase();
            return lower === '复制' || lower.includes('复制') || lower.includes('拷贝') || lower === 'copy' || lower.includes('copy');
        }
        function isEditLike(text) {
            const lower = normalizeText(text).toLowerCase();
            return lower === '编辑' || lower.includes('编辑') || lower.includes('修改') || lower === 'edit' || lower.includes('edit');
        }
        function isNoDataText(text) {
            const lower = normalizeText(text).toLowerCase();
            return lower.includes('暂无数据') || lower.includes('no data') || lower.includes('no records') || lower.includes('empty');
        }
        function hasTokenActions(row) {
            if (!row) return false;
            const texts = Array.from(row.querySelectorAll('button, a, [role="button"], span, div, i'))
                .map((node) => normalizeText(node.innerText || node.textContent || ''))
                .filter((text) => !!text);
            return texts.some((text) => isCopyLike(text)) && texts.some((text) => isEditLike(text));
        }
        function hasActionControls(row) {
            if (!row) return false;
            const controls = Array.from(row.querySelectorAll('button, a, [role="button"], span, div, i')).filter((node) => {
                if (!isVisible(node)) return false;
                const text = normalizeText(node.innerText || node.textContent || '');
                const cls = String(node.className || '').toLowerCase();
                const role = String(node.getAttribute ? (node.getAttribute('role') || '') : '').toLowerCase();
                return !!text || cls.includes('icon') || cls.includes('more') || cls.includes('action') || role === 'button';
            });
            return controls.length >= 2;
        }
        function isLikelyTokenRow(row) {
            const text = normalizeText(row ? row.innerText : '');
            if (!text) return false;
            if (row && row.querySelector('th')) return false;
            if (isNoDataText(text)) return false;
            if (hasTokenActions(row)) return true;
            const lower = text.toLowerCase();
            const columnCount = row ? row.querySelectorAll('td').length : 0;
            if (hasActionControls(row) && columnCount >= 4) return true;
            return (
                (text.includes('已启用') || lower.includes('enabled')) &&
                (text.includes('用户分组') || lower.includes('group')) &&
                isEditLike(text)
            );
        }
        function collectEditCandidates(root) {
            const exact = [];
            const fuzzy = [];
            const nodes = Array.from(root.querySelectorAll('button, a, [role="button"], span, div'));
            for (const node of nodes) {
                const text = normalizeText(node.innerText || node.textContent || '');
                if (!text || !isEditLike(text) || !isVisible(node)) continue;
                const hasChildExactEdit = Array.from(node.querySelectorAll('*')).some((child) => {
                    return isEditLike(normalizeText(child.innerText || child.textContent || ''));
                });
                const exactText = normalizeText(text).toLowerCase() === '编辑' || normalizeText(text).toLowerCase() === 'edit';
                if (hasChildExactEdit && !exactText) continue;
                const clickable = toClickable(node);
                if (!isVisible(clickable) || !isEnabled(clickable)) continue;
                const clickableText = normalizeText(clickable.innerText || clickable.textContent || '');
                const exactClickable =
                    clickableText.toLowerCase() === '编辑' || clickableText.toLowerCase() === 'edit';
                const hasCopyAndEdit =
                    (isCopyLike(text) || isCopyLike(clickableText)) &&
                    (isEditLike(text) || isEditLike(clickableText));
                if (hasCopyAndEdit && !exactText && !exactClickable) continue;
                if (isCopyLike(text) && !exactText) continue;
                if (isCopyLike(clickableText) && !exactClickable) continue;
                const bucket = (exactText || exactClickable) ? exact : fuzzy;
                if (!bucket.includes(clickable)) bucket.push(clickable);
            }
            return exact.concat(fuzzy);
        }
        function clickWithEvents(node) {
            if (!node) return { clicked: false, reason: 'no_target' };
            node.scrollIntoView({ block: 'center', inline: 'center' });
            const rect = node.getBoundingClientRect();
            const x = Math.floor(rect.left + rect.width / 2);
            const y = Math.floor(rect.top + rect.height / 2);
            const target = node;
            const events = ['pointerover', 'mouseover', 'pointerdown', 'mousedown', 'pointerup', 'mouseup', 'click'];
            for (const name of events) {
                const Ctor = name.startsWith('pointer') ? PointerEvent : MouseEvent;
                target.dispatchEvent(new Ctor(name, {
                    bubbles: true,
                    cancelable: true,
                    view: window,
                    clientX: x,
                    clientY: y
                }));
            }
            if (typeof node.click === 'function') node.click();
            return {
                clicked: true,
                reason: 'row_direct'
            };
        }
        const rows = Array.from(
            document.querySelectorAll('tbody tr, .semi-table-tbody .semi-table-row, .semi-table-row')
        ).filter((row) => isVisible(row) && isLikelyTokenRow(row));
        const row = rows.length ? rows[0] : null;
        if (!row) return { clicked: false, reason: 'no_token_row' };
        row.scrollIntoView({ block: 'center', inline: 'nearest' });
        row.dispatchEvent(new MouseEvent('mouseenter', { bubbles: true, cancelable: true, view: window }));
        row.dispatchEvent(new MouseEvent('mouseover', { bubbles: true, cancelable: true, view: window }));
        const candidates = collectEditCandidates(row);
        if (!candidates.length) {
            return { clicked: false, reason: 'row_no_direct_edit' };
        }
        return clickWithEvents(candidates[0]);
    "#;

    // 等待令牌行出现(最多8秒轮询，失败则刷新重试一次)
    let mut has_token_row = false;
    let mut token_list_empty = false;
    for wait_round in 1..=2 {
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(8) {
            match driver.execute(wait_row_script, Vec::<Value>::new()).await {
                Ok(result) => {
                    let payload = result.json();
                    let has_row = if payload.is_object() {
                        payload
                            .get("hasTokenRow")
                            .and_then(Value::as_bool)
                            .unwrap_or(false)
                    } else {
                        payload.as_bool().unwrap_or(false)
                    };
                    if has_row {
                        has_token_row = true;
                        break;
                    }
                    if payload.is_object() {
                        token_list_empty = payload
                            .get("hasEmptyState")
                            .and_then(Value::as_bool)
                            .unwrap_or(false);
                    }
                    async_sleep(Duration::from_millis(200)).await;
                }
                Err(err) => anyhow::bail!("检测令牌列表失败: {err}"),
            }
        }
        if has_token_row {
            break;
        }
        if wait_round == 1 {
            tracing::debug!("首次等待未发现可编辑令牌，刷新页面后重试");
            if let Err(err) = driver.refresh().await {
                tracing::debug!("刷新令牌页失败: {}", err);
            }
            async_sleep(Duration::from_millis(1200)).await;
        }
    }
    if !has_token_row {
        if token_list_empty {
            anyhow::bail!("令牌列表为空，暂无可同步 API Key");
        }
        anyhow::bail!("未找到可编辑的令牌");
    }

    // 执行首行直点编辑(单次尝试)
    let clicked_ret = driver
        .execute(direct_click_script, Vec::<Value>::new())
        .await
        .map_err(|err| anyhow::anyhow!("点击编辑按钮失败: {err}"))?;
    let obj = clicked_ret.json().clone();
    let clicked = obj.get("clicked").and_then(Value::as_bool).unwrap_or(false);
    if !clicked {
        let reason = obj
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        anyhow::bail!("首行直点编辑未命中: {}", reason);
    }

    // 等待编辑弹窗出现(最多2秒)
    let open_started = Instant::now();
    while open_started.elapsed() < Duration::from_secs(2) {
        let opened = driver
            .execute(editor_open_script, Vec::<Value>::new())
            .await
            .map_err(|err| anyhow::anyhow!("检测编辑弹窗状态失败: {err}"))?;
        if opened.json().as_bool().unwrap_or(false) {
            return Ok(());
        }
        async_sleep(Duration::from_millis(180)).await;
    }
    anyhow::bail!("首行直点编辑已点击但弹窗未出现")
}

async fn detect_quota_unit_rate(driver: &WebDriver) -> Result<f64> {
    let script = r#"
        function isVisible(node) {
            if (!node) return false;
            const style = window.getComputedStyle(node);
            if (style.display === 'none' || style.visibility === 'hidden') return false;
            const rect = node.getBoundingClientRect();
            return rect.width > 0 && rect.height > 0;
        }
        const roots = Array.from(document.querySelectorAll(
            '.semi-modal-content, .semi-modal, .semi-sidesheet, .semi-sidesheet-content, .semi-sideSheet, [class*="sidesheet"], [class*="sideSheet"], [role="dialog"]'
        ));
        let root = roots.find((item) => item && isVisible(item) && (
            (item.innerText || '').includes('更新令牌信息') || (item.innerText || '').includes('额度设置')
        ));
        if (!root) {
            root = roots.find((item) => item && isVisible(item));
        }
        if (!root) {
            root = document.body;
        }
        const text = root.innerText || '';
        const amountMatch = text.match(/等价金额[:：]\s*\$\s*(-?[\d,.]+)/);
        const amountValue = amountMatch ? Number((amountMatch[1] || '').replace(/,/g, '')) : null;

        const labels = Array.from(root.querySelectorAll('*')).filter((el) => {
            const t = (el.textContent || '').trim();
            return t === '额度';
        });
        function findInput(startNode) {
            let node = startNode;
            for (let i = 0; i < 6 && node; i += 1) {
                const parent = node.parentElement;
                if (!parent) break;
                const input = parent.querySelector('input');
                if (input && isVisible(input)) return input;
                node = parent;
            }
            return null;
        }
        let quotaValue = null;
        for (const label of labels) {
            const input = findInput(label);
            if (!input) continue;
            const raw = (input.value || '').replace(/,/g, '').trim();
            if (!raw) continue;
            const num = Number(raw);
            if (!Number.isNaN(num)) {
                quotaValue = num;
                break;
            }
        }
        return {quotaValue, amountValue};
    "#;
    let value = driver.execute(script, Vec::<Value>::new()).await?;
    let obj = value.json();
    let quota = obj.get("quotaValue").and_then(to_f64);
    let amount = obj.get("amountValue").and_then(to_f64);
    match (quota, amount) {
        (Some(q), Some(a)) if a.abs() > f64::EPSILON => {
            let rate = (q / a).abs();
            if (1000.0..=10000000.0).contains(&rate) {
                Ok(rate)
            } else {
                Ok(QUOTA_UNIT_PER_DOLLAR)
            }
        }
        _ => Ok(QUOTA_UNIT_PER_DOLLAR),
    }
}

async fn set_modal_quota_value(driver: &WebDriver, quota_value: i64) -> Result<()> {
    let script = r#"
        const targetQuota = String(arguments[0]);
        function isVisible(node) {
            if (!node) return false;
            const style = window.getComputedStyle(node);
            if (style.display === 'none' || style.visibility === 'hidden') return false;
            const rect = node.getBoundingClientRect();
            return rect.width > 0 && rect.height > 0;
        }
        function normalizeText(text) {
            return String(text || '').replace(/\s+/g, ' ').trim();
        }
        function normalizeDigits(text) {
            return String(text || '').replace(/[,\s]/g, '').trim();
        }
        function locateRoot() {
            const roots = Array.from(document.querySelectorAll(
                '.semi-modal-content, .semi-modal, .semi-sidesheet, .semi-sidesheet-content, .semi-sideSheet, [class*="sidesheet"], [class*="sideSheet"], [role="dialog"]'
            ));
            let root = roots.find((item) => item && isVisible(item) && (
                (item.innerText || '').includes('更新令牌信息') || (item.innerText || '').includes('额度设置')
            ));
            if (!root) {
                root = roots.find((item) => item && isVisible(item));
            }
            return root || document.body;
        }
        function isWritableInput(input) {
            if (!input) return false;
            if (!isVisible(input)) return false;
            if (input.disabled) return false;
            const type = String(input.type || '').toLowerCase();
            if (type === 'hidden') return false;
            return true;
        }
        function addCandidate(list, input, strategy) {
            if (!isWritableInput(input)) return;
            if (!list.some((item) => item.input === input)) {
                list.push({ input, strategy });
            }
        }
        function collectCandidates(root) {
            const list = [];
            const labels = Array.from(root.querySelectorAll('*')).filter((el) => {
                return normalizeText(el.textContent || '') === '额度';
            });
            for (const label of labels) {
                let node = label;
                for (let i = 0; i < 8 && node; i += 1) {
                    const parent = node.parentElement;
                    if (!parent) break;
                    const input = parent.querySelector('input');
                    if (input) {
                        addCandidate(list, input, 'label_quota');
                    }
                    node = parent;
                }
            }
            const semanticInputs = Array.from(root.querySelectorAll('input')).filter((input) => {
                if (!isWritableInput(input)) return false;
                const haystack = [
                    input.getAttribute('placeholder') || '',
                    input.getAttribute('name') || '',
                    input.getAttribute('id') || '',
                    input.getAttribute('aria-label') || '',
                    input.className || ''
                ].join(' ').toLowerCase();
                return (
                    haystack.includes('额度') ||
                    haystack.includes('quota') ||
                    haystack.includes('limit')
                );
            });
            for (const input of semanticInputs) {
                addCandidate(list, input, 'semantic');
            }
            const fallbackInputs = Array.from(root.querySelectorAll('input'));
            for (const input of fallbackInputs) {
                addCandidate(list, input, 'fallback');
            }
            return list;
        }
        function writeInputValue(input, value) {
            try {
                input.removeAttribute('readonly');
            } catch (e) {}
            const descriptor = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value');
            if (descriptor && descriptor.set) {
                descriptor.set.call(input, value);
            } else {
                input.value = value;
            }
            input.dispatchEvent(new Event('input', { bubbles: true }));
            input.dispatchEvent(new Event('change', { bubbles: true }));
            input.dispatchEvent(new Event('blur', { bubbles: true }));
            input.dispatchEvent(new KeyboardEvent('keyup', {
                bubbles: true,
                key: 'Enter',
                code: 'Enter'
            }));
        }
        const root = locateRoot();
        const candidates = collectCandidates(root);
        if (!candidates.length) {
            return {
                ok: false,
                reason: 'quota_input_not_found',
                candidateCount: 0
            };
        }

        const targetDigits = normalizeDigits(targetQuota);
        const tried = [];
        for (let idx = 0; idx < candidates.length; idx += 1) {
            const item = candidates[idx];
            const input = item.input;
            try {
                input.focus();
            } catch (e) {}
            writeInputValue(input, targetQuota);
            const currentDigits = normalizeDigits(input.value || '');
            const currentText = normalizeText(input.value || '');
            tried.push({
                index: idx + 1,
                strategy: item.strategy,
                value: currentText
            });
            if (currentDigits === targetDigits) {
                return {
                    ok: true,
                    reason: 'written',
                    strategy: item.strategy,
                    index: idx + 1,
                    value: currentText,
                    candidateCount: candidates.length,
                    tried
                };
            }
        }
        return {
            ok: false,
            reason: 'write_verify_failed',
            candidateCount: candidates.length,
            tried
        };
    "#;
    let value = driver
        .execute(script, vec![Value::from(quota_value)])
        .await?;
    let result = value.json().clone();
    if !result.is_object() {
        anyhow::bail!("额度输入返回异常结果");
    }
    if !result.get("ok").and_then(Value::as_bool).unwrap_or(false) {
        let reason = result
            .get("reason")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        anyhow::bail!("填写额度失败: {}", reason);
    }
    Ok(())
}

async fn submit_quota_modal(driver: &WebDriver) -> Result<()> {
    let script = r#"
        function isVisible(node) {
            if (!node) return false;
            const style = window.getComputedStyle(node);
            if (style.display === 'none' || style.visibility === 'hidden') return false;
            const rect = node.getBoundingClientRect();
            return rect.width > 0 && rect.height > 0;
        }
        const roots = Array.from(document.querySelectorAll(
            '.semi-modal-content, .semi-modal, .semi-sidesheet, .semi-sidesheet-content, .semi-sideSheet, [class*="sidesheet"], [class*="sideSheet"], [role="dialog"]'
        ));
        let root = roots.find((item) => item && isVisible(item) && (
            (item.innerText || '').includes('更新令牌信息') || (item.innerText || '').includes('额度设置')
        ));
        if (!root) {
            root = roots.find((item) => item && isVisible(item));
        }
        if (!root) {
            root = document.body;
        }
        const btn = Array.from(root.querySelectorAll('button')).find((node) => {
            const text = (node.innerText || node.textContent || '').trim();
            return text.includes('提交') && isVisible(node) && !node.disabled;
        });
        if (!btn) return false;
        btn.click();
        return true;
    "#;
    let clicked = driver.execute(script, Vec::<Value>::new()).await?;
    if !clicked.json().as_bool().unwrap_or(false) {
        anyhow::bail!("未找到提交按钮");
    }

    let check_script = r#"
        function isVisible(node) {
            if (!node) return false;
            const style = window.getComputedStyle(node);
            if (style.display === 'none' || style.visibility === 'hidden') return false;
            const rect = node.getBoundingClientRect();
            return rect.width > 0 && rect.height > 0;
        }
        const roots = Array.from(document.querySelectorAll(
            '.semi-modal-content, .semi-modal, .semi-sidesheet, .semi-sidesheet-content, .semi-sideSheet, [class*="sidesheet"], [class*="sideSheet"], [role="dialog"]'
        )).filter(isVisible);
        return roots.some((root) => {
            const text = root.innerText || '';
            return text.includes('更新令牌信息') || text.includes('额度设置') || text.includes('编辑令牌');
        });
    "#;
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(8) {
        let still_open = driver.execute(check_script, Vec::<Value>::new()).await?;
        if !still_open.json().as_bool().unwrap_or(false) {
            return Ok(());
        }
        async_sleep(Duration::from_millis(220)).await;
    }
    anyhow::bail!("提交后弹窗未关闭")
}
