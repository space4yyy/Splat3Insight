use tauri::AppHandle;
use tauri_plugin_notification::NotificationExt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AuthFailure {
    NintendoLogin,
    SplatNetAuthorization,
    NintendoSessionExpired,
}

impl AuthFailure {
    // 系统通知可能显示在锁屏上，只使用固定文案，不带账号信息、原始错误或登录链接。
    fn content(self) -> (&'static str, &'static str) {
        match self {
            Self::NintendoLogin => (
                "Nintendo 账号登录失败",
                "请打开 Splat3 Insight 查看失败原因，并重新获取登录链接后重试。本地历史数据不受影响。",
            ),
            Self::SplatNetAuthorization => (
                "SplatNet 授权失败",
                "请打开 Splat3 Insight 查看失败原因，检查网络或稍后重新获取权限。本地历史数据不受影响。",
            ),
            Self::NintendoSessionExpired => (
                "Nintendo 登录状态已失效",
                "请打开 Splat3 Insight，重新登录 Nintendo 账号后获取 SplatNet 权限。本地历史数据不受影响。",
            ),
        }
    }
}

pub(super) fn notify(app: &AppHandle, failure: AuthFailure) {
    let (title, body) = failure.content();
    if let Err(error) = app.notification().builder().title(title).body(body).show() {
        super::log_console_error(
            "auth-notification",
            &format!("发送授权失败系统通知失败：{error}"),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::AuthFailure;

    #[test]
    fn failures_explain_the_action_without_exposing_account_details() {
        for failure in [
            AuthFailure::NintendoLogin,
            AuthFailure::SplatNetAuthorization,
            AuthFailure::NintendoSessionExpired,
        ] {
            let (title, body) = failure.content();
            assert!(!title.is_empty());
            assert!(body.contains("请打开 Splat3 Insight"));
            assert!(body.contains("本地历史数据不受影响"));
            assert!(!body.contains("token"));
            assert!(!body.contains("://"));
        }
    }

    #[test]
    fn expired_login_requires_signing_in_but_transient_authorization_failure_does_not() {
        let (_, expired) = AuthFailure::NintendoSessionExpired.content();
        assert!(expired.contains("重新登录 Nintendo 账号"));
        let (_, transient) = AuthFailure::SplatNetAuthorization.content();
        assert!(transient.contains("重新获取权限"));
        assert!(!transient.contains("重新登录"));
    }
}
