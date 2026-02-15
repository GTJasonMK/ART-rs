use crate::utils::{parse_first_number, to_f64};
use anyhow::{Context, Result};
use chrono::{Datelike, Local, NaiveDate};
use reqwest::header::{ACCEPT, AUTHORIZATION, HeaderMap, HeaderValue};
use serde_json::Value;

const QUOTA_UNIT_PER_DOLLAR: f64 = 500000.0;

#[derive(Debug, Clone)]
pub struct ApiBalanceResult {
    pub success: bool,
    pub balance: Option<f64>,
    pub source: String,
    pub message: String,
}

impl ApiBalanceResult {
    pub fn ok(balance: f64, source: &str, message: &str) -> Self {
        Self {
            success: true,
            balance: Some(balance),
            source: source.to_string(),
            message: message.to_string(),
        }
    }

    pub fn fail(message: impl Into<String>) -> Self {
        Self {
            success: false,
            balance: None,
            source: String::new(),
            message: message.into(),
        }
    }
}

#[derive(Clone)]
pub struct ApiBalanceClient {
    base_url: String,
    client: reqwest::Client,
}

impl ApiBalanceClient {
    pub fn new(base_url: &str, timeout_seconds: u64) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(timeout_seconds.max(1)))
            .build()
            .with_context(|| "创建HTTP客户端失败")?;
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            client,
        })
    }

    pub async fn query_balance(&self, api_key: &str) -> ApiBalanceResult {
        let key = api_key.trim();
        if key.is_empty() {
            return ApiBalanceResult::fail("缺少 API Key");
        }

        let headers = match build_headers(key) {
            Ok(item) => item,
            Err(e) => return ApiBalanceResult::fail(format!("构造请求头失败: {e}")),
        };

        match self.query_via_billing_routes(&headers).await {
            Ok(balance) => ApiBalanceResult::ok(
                balance,
                "billing:subscription+usage",
                "通过账单路由计算余额",
            ),
            Err(e) => {
                tracing::debug!("账单路由查询失败，开始尝试兼容路由: {e}");

                let today = Local::now().date_naive();
                let month_start = first_day_of_month(today).unwrap_or(today);
                let candidates = vec![
                    format!(
                        "/v1/dashboard/billing/usage?start_date={}&end_date={}",
                        month_start, today
                    ),
                    "/v1/dashboard/billing/subscription".to_string(),
                    "/v1/dashboard/billing/credit_grants".to_string(),
                    "/dashboard/billing/credit_grants".to_string(),
                    "/api/user/balance".to_string(),
                    "/api/user/self".to_string(),
                    "/api/user/info".to_string(),
                    "/api/token/self".to_string(),
                    "/api/token/info".to_string(),
                    "/v1/models".to_string(),
                ];

                let mut last_error = "未命中可用余额接口".to_string();
                for path in candidates {
                    let url = format!("{}{}", self.base_url, path);
                    let response = match self.client.get(&url).headers(headers.clone()).send().await
                    {
                        Ok(item) => item,
                        Err(req_err) => {
                            last_error = format!("请求异常({}): {}", path, req_err);
                            tracing::debug!("{}", last_error);
                            continue;
                        }
                    };

                    let status = response.status().as_u16();
                    if status >= 400 {
                        last_error = format!("HTTP {} ({})", status, path);
                        tracing::debug!("{}", last_error);
                        continue;
                    }

                    if let Some(header_value) = extract_balance_from_headers(response.headers()) {
                        return ApiBalanceResult::ok(
                            header_value.max(0.0),
                            &format!("header:{}", path),
                            "通过响应头获取余额",
                        );
                    }

                    let body_text = match response.text().await {
                        Ok(text) => text,
                        Err(parse_err) => {
                            last_error = format!("读取响应体失败({}): {}", path, parse_err);
                            tracing::debug!("{}", last_error);
                            continue;
                        }
                    };

                    if let Some(body_value) = extract_balance_from_body(&body_text) {
                        return ApiBalanceResult::ok(
                            body_value.max(0.0),
                            &format!("body:{}", path),
                            "通过响应体获取余额",
                        );
                    }

                    last_error = format!("接口无可解析余额字段({})", path);
                }

                ApiBalanceResult::fail(last_error)
            }
        }
    }

    async fn query_via_billing_routes(&self, headers: &HeaderMap) -> Result<f64> {
        let today = Local::now().date_naive();
        let month_start = first_day_of_month(today)?;
        let sub_url = format!("{}/v1/dashboard/billing/subscription", self.base_url);
        let usage_url = format!(
            "{}/v1/dashboard/billing/usage?start_date={}&end_date={}",
            self.base_url, month_start, today
        );

        let (sub_resp, usage_resp) = tokio::try_join!(
            self.client.get(&sub_url).headers(headers.clone()).send(),
            self.client.get(&usage_url).headers(headers.clone()).send(),
        )
        .with_context(|| "请求账单路由失败")?;

        if sub_resp.status().as_u16() >= 400 || usage_resp.status().as_u16() >= 400 {
            anyhow::bail!(
                "账单路由HTTP异常: subscription={},usage={}",
                sub_resp.status().as_u16(),
                usage_resp.status().as_u16()
            );
        }

        let sub_json: Value = sub_resp
            .json()
            .await
            .with_context(|| "解析 subscription JSON 失败")?;
        let usage_json: Value = usage_resp
            .json()
            .await
            .with_context(|| "解析 usage JSON 失败")?;

        let hard_limit = to_f64(
            sub_json
                .get("hard_limit_usd")
                .or(sub_json.get("soft_limit_usd")),
        )
        .with_context(|| "subscription 缺少 hard_limit_usd/soft_limit_usd")?;
        let total_usage =
            to_f64(usage_json.get("total_usage")).with_context(|| "usage 缺少 total_usage")?;

        let usage_usd = if hard_limit > 0.0 && total_usage > hard_limit * 2.0 {
            total_usage / 100.0
        } else {
            total_usage
        };
        let remain = (hard_limit - usage_usd).max(0.0);

        tracing::debug!(
            "账单路由余额计算: hard_limit_usd={:.4}, total_usage_raw={:.4}, usage_usd={:.4}, remain={:.4}",
            hard_limit,
            total_usage,
            usage_usd,
            remain
        );
        Ok(remain)
    }
}

fn build_headers(api_key: &str) -> Result<HeaderMap> {
    let mut headers = HeaderMap::new();
    let auth = format!("Bearer {}", api_key.trim());
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&auth).with_context(|| "Authorization 值非法")?,
    );
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    Ok(headers)
}

fn extract_balance_from_headers(headers: &HeaderMap) -> Option<f64> {
    let usd_keys = [
        "x-balance",
        "x-user-balance",
        "x-credit-balance",
        "x-remaining-balance",
        "x-total-available",
        "x-account-balance",
    ];
    let quota_keys = ["x-quota", "x-remaining-quota", "x-total-quota"];

    for key in usd_keys {
        if let Some(raw) = get_header_text(headers, key) {
            if let Some(value) = parse_first_number(&raw) {
                return Some(value.max(0.0));
            }
        }
    }

    for key in quota_keys {
        if let Some(raw) = get_header_text(headers, key) {
            if let Some(value) = parse_first_number(&raw) {
                return Some((value / QUOTA_UNIT_PER_DOLLAR).max(0.0));
            }
        }
    }

    None
}

fn get_header_text(headers: &HeaderMap, target: &str) -> Option<String> {
    for (key, value) in headers {
        if key.as_str().eq_ignore_ascii_case(target) {
            return value.to_str().ok().map(|item| item.to_string());
        }
    }
    None
}

fn extract_balance_from_body(text: &str) -> Option<f64> {
    let data: Value = serde_json::from_str(text).ok()?;

    if let Some(value) = to_f64(data.get("total_available")) {
        return Some(value.max(0.0));
    }

    if let Some(value) = to_f64(data.get("balance")) {
        return Some(normalize_balance_value(value, "balance").max(0.0));
    }

    scan_balance_value(&data, 0).map(|item| item.max(0.0))
}

fn scan_balance_value(obj: &Value, depth: usize) -> Option<f64> {
    if depth > 5 {
        return None;
    }

    let usd_field_patterns = [
        "balance",
        "remaining_balance",
        "available_balance",
        "current_balance",
        "credit_balance",
        "total_available",
        "available_credit",
        "remain_amount",
    ];
    let quota_field_patterns = [
        "quota",
        "remaining_quota",
        "remain_quota",
        "left_quota",
        "available_quota",
    ];

    match obj {
        Value::Object(map) => {
            for (key, value) in map {
                let lower = key.to_ascii_lowercase();
                if usd_field_patterns.iter().any(|item| lower.contains(item)) {
                    if let Some(parsed) = to_f64(Some(value)) {
                        return Some(normalize_balance_value(parsed, &lower));
                    }
                }
            }

            for (key, value) in map {
                let lower = key.to_ascii_lowercase();
                if quota_field_patterns.iter().any(|item| lower.contains(item)) {
                    if let Some(parsed) = to_f64(Some(value)) {
                        return Some(parsed / QUOTA_UNIT_PER_DOLLAR);
                    }
                }
            }

            for value in map.values() {
                if let Some(found) = scan_balance_value(value, depth + 1) {
                    return Some(found);
                }
            }
            None
        }
        Value::Array(items) => {
            for item in items {
                if let Some(found) = scan_balance_value(item, depth + 1) {
                    return Some(found);
                }
            }
            None
        }
        _ => None,
    }
}

fn normalize_balance_value(value: f64, key_hint: &str) -> f64 {
    if key_hint.contains("quota") {
        return value / QUOTA_UNIT_PER_DOLLAR;
    }
    if value.abs() > 100000.0 {
        return value / QUOTA_UNIT_PER_DOLLAR;
    }
    value
}


fn first_day_of_month(today: NaiveDate) -> Result<NaiveDate> {
    NaiveDate::from_ymd_opt(today.year(), today.month(), 1).with_context(|| "计算月初日期失败")
}
