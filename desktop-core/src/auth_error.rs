//! Only error fields cross the authentication boundary; token payloads never do.
use serde_json::Value;
use std::sync::OnceLock;

pub(crate) fn provider_detail(value: &Value) -> String {
    fn collect(value: &Value, depth: usize, fields: &mut Vec<String>) {
        if depth > 2 {
            return;
        }
        for key in [
            "error",
            "code",
            "status",
            "reason",
            "message",
            "errorMessage",
            "error_description",
        ] {
            let Some(value) = value.get(key) else {
                continue;
            };
            match value {
                Value::String(text) if safe_detail(text) => {
                    fields.push(format!("{key}={}", text.trim()))
                }
                Value::Number(number) => fields.push(format!("{key}={number}")),
                Value::Object(_) => collect(value, depth + 1, fields),
                _ => {}
            }
        }
    }
    let mut fields = Vec::new();
    collect(value, 0, &mut fields);
    if fields.is_empty() {
        "服务返回错误".to_owned()
    } else {
        fields.join("；")
    }
}

fn safe_detail(text: &str) -> bool {
    let text = text.trim();
    let normalized = text.to_ascii_lowercase();
    static CREDENTIAL_ASSIGNMENT: OnceLock<regex::Regex> = OnceLock::new();
    let assignment = CREDENTIAL_ASSIGNMENT.get_or_init(|| {
        regex::Regex::new(r#"(?i)(token|session|cookie|password|secret|authorization|verifier|\bf)\s*["']?\s*[:=]"#).unwrap()
    });
    // “invalid_token” / “token expired” are useful errors, not credentials.
    // Reject echoed values, URLs, JWTs and opaque credential strings instead.
    !text.is_empty()
        && text.len() <= 512
        && !text.chars().any(char::is_control)
        && !assignment.is_match(text)
        && ![
            "token=",
            "token:",
            "cookie",
            "password",
            "secret",
            "authorization",
            "bearer ",
            "session=",
            "session:",
            "verifier",
            "f=",
            "://",
            "eyj",
        ]
        .iter()
        .any(|hint| normalized.contains(hint))
        && !text.split_whitespace().any(|word| {
            word.len() > 32
                && word
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_.+/=".contains(c))
        })
}

pub(crate) fn provider_error(service: &str, status: u16, value: &Value) -> String {
    format!("{service}失败（HTTP {status}）：{}", provider_detail(value))
}

pub(crate) fn parse_response(
    service: &str,
    status: reqwest::StatusCode,
    body: &str,
) -> Result<Value, String> {
    let parsed = serde_json::from_str::<Value>(body);
    if !status.is_success() {
        let value = parsed.unwrap_or_else(|_| {
            // A short plain-text error is useful too; HTML gateway pages aren't.
            if !body.contains(['<', '>']) {
                serde_json::json!({ "message": body })
            } else {
                Value::Null
            }
        });
        return Err(provider_error(service, status.as_u16(), &value));
    }
    parsed.map_err(|error| format!("{service}返回了无效 JSON（HTTP {status}）：{error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn preserves_error_code_and_description_including_token_errors() {
        let error = provider_error(
            "nxapi 授权",
            401,
            &json!({
                "error": "invalid_token", "error_description": "The access token has expired"
            }),
        );
        assert!(error.contains("HTTP 401"));
        assert!(error.contains("error=invalid_token"));
        assert!(error.contains("The access token has expired"));
    }

    #[test]
    fn preserves_nested_errors_and_coral_business_status() {
        let error = provider_error(
            "Nintendo Coral",
            200,
            &json!({
                "status": 9403, "errorMessage": "Membership required",
                "error": { "code": "access_denied", "message": "Not permitted" }
            }),
        );
        for detail in [
            "9403",
            "Membership required",
            "access_denied",
            "Not permitted",
        ] {
            assert!(error.contains(detail));
        }
    }

    #[test]
    fn does_not_expose_credentials_or_unrelated_response_fields() {
        let error = provider_error(
            "nxapi",
            400,
            &json!({
                "error": "invalid_request", "access_token": "private-access",
                "session_token": "private-session", "result": { "token": "private-result" },
                "message": "token=private-echo", "error_description": "https://example.com/?code=private-code"
            }),
        );
        assert!(error.contains("invalid_request"));
        assert!(!error.contains("private-"));
    }

    #[test]
    fn echoed_credentials_with_spaces_or_quotes_are_also_hidden() {
        for message in [
            "access_token = private-value",
            "session_token: private-value",
            r#"access_token": "private-value"#,
            "Bearer private-value",
            "provider echoed eyJhbGciOiJIUzI1NiJ9.private-value.signature",
        ] {
            let error = provider_error("nxapi", 400, &json!({ "message": message }));
            assert!(!error.contains("private-value"));
        }
    }

    #[test]
    fn non_json_failures_keep_http_status_and_safe_plain_text() {
        let error = parse_response(
            "nxapi",
            reqwest::StatusCode::BAD_GATEWAY,
            "Service unavailable",
        )
        .unwrap_err();
        assert!(error.contains("HTTP 502"));
        assert!(error.contains("Service unavailable"));
        let html = parse_response(
            "nxapi",
            reqwest::StatusCode::BAD_GATEWAY,
            "<html>gateway</html>",
        )
        .unwrap_err();
        assert!(html.contains("HTTP 502"));
        assert!(!html.contains("<html>"));
    }
}
