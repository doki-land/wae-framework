//! 参数脱敏：写入审计记录前统一替换凭据、私钥、密码与 token。

use serde_json::Value;

/// 敏感值统一替换后的文本。
pub const REDACTED: &str = "<redacted>";

const MAX_DEPTH: usize = 8;
const MAX_STRING_CHARS: usize = 1024;
const MAX_DEPTH_MARKER: &str = "<truncated>";

const SENSITIVE_KEYS: [&str; 14] = [
    "password",
    "passwd",
    "passphrase",
    "secret",
    "token",
    "apikey",
    "credential",
    "authorization",
    "cookie",
    "privatekey",
    "privkey",
    "accesskey",
    "sessionkey",
    "bearer",
];

/// 对业务参数递归脱敏，返回可安全写入的记录值。
pub fn sanitize_params(params: Value) -> Value {
    sanitize(params, 0)
}

fn sanitize(value: Value, depth: usize) -> Value {
    if depth > MAX_DEPTH {
        return Value::String(MAX_DEPTH_MARKER.into());
    }
    match value {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(
                    |(key, value)| {
                        if is_sensitive_key(&key) { (key, Value::String(REDACTED.into())) } else { (key, sanitize(value, depth + 1)) }
                    },
                )
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.into_iter().map(|item| sanitize(item, depth + 1)).collect()),
        Value::String(text) => Value::String(sanitize_text(&text)),
        other => other,
    }
}

fn is_sensitive_key(key: &str) -> bool {
    let normalized: String = key.chars().filter(|c| !matches!(c, '_' | '-' | ' ')).flat_map(char::to_lowercase).collect();
    SENSITIVE_KEYS.iter().any(|needle| normalized.contains(needle))
}

pub(crate) fn sanitize_text(text: &str) -> String {
    if text.contains("-----BEGIN") && text.contains("PRIVATE KEY") {
        return REDACTED.into();
    }
    if let Some(rest) = text.strip_prefix("Bearer ") {
        if rest.is_empty() {
            return text.to_string();
        }
        return format!("Bearer {REDACTED}");
    }
    truncate(text)
}

fn truncate(text: &str) -> String {
    if text.chars().count() <= MAX_STRING_CHARS {
        return text.to_string();
    }
    let mut out: String = text.chars().take(MAX_STRING_CHARS - 1).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn sensitive_keys_are_redacted_at_any_depth() {
        let value = sanitize_params(json!({
            "token": "TOKENVALUE",
            "apiKey": "APIKEYVALUE",
            "nested": { "authorization": "NESTEDVALUE", "list": [{ "password": "PASSWORDVALUE" }] },
            "safe": "keep-me",
        }));
        let text = value.to_string();
        assert!(!text.contains("TOKENVALUE"));
        assert!(!text.contains("APIKEYVALUE"));
        assert!(!text.contains("NESTEDVALUE"));
        assert!(!text.contains("PASSWORDVALUE"));
        assert_eq!(value["safe"], "keep-me");
        assert_eq!(value["token"], REDACTED);
        assert_eq!(value["nested"]["list"][0]["password"], REDACTED);
    }

    #[test]
    fn private_key_and_bearer_values_are_redacted() {
        let value = sanitize_params(json!({
            "key": "-----BEGIN RSA PRIVATE KEY-----\nKEYMATERIAL\n-----END RSA PRIVATE KEY-----",
            "header": "Bearer BEARERVALUE",
        }));
        assert_eq!(value["key"], REDACTED);
        assert_eq!(value["header"], format!("Bearer {REDACTED}"));
        assert!(!value.to_string().contains("KEYMATERIAL"));
        assert!(!value.to_string().contains("BEARERVALUE"));
    }

    #[test]
    fn long_strings_are_truncated() {
        let long = "x".repeat(MAX_STRING_CHARS + 10);
        let value = sanitize_params(json!({ "blob": long }));
        let text = value["blob"].as_str().unwrap();
        assert!(text.chars().count() <= MAX_STRING_CHARS);
        assert!(text.ends_with('…'));
    }
}
