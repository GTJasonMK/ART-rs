use regex::Regex;
use serde_json::Value;
use std::sync::OnceLock;

fn balance_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"-?[\d,]+(?:\.\d+)?").unwrap())
}

/// 从文本中提取第一个数字（支持逗号分隔和小数）
pub fn parse_first_number(text: &str) -> Option<f64> {
    let matched = balance_regex().find(text)?.as_str().replace(',', "");
    matched.parse::<f64>().ok()
}

/// 从 serde_json::Value（Option 包装）提取 f64，支持 Number 和 String 类型
pub fn to_f64(value: Option<&Value>) -> Option<f64> {
    match value {
        Some(Value::Number(item)) => item.as_f64(),
        Some(Value::String(text)) => text.replace(',', "").parse::<f64>().ok(),
        _ => None,
    }
}

/// 从 serde_json::Value 引用提取 f64，用于 .and_then(value_to_f64) 场景
pub fn value_to_f64(value: &Value) -> Option<f64> {
    to_f64(Some(value))
}
