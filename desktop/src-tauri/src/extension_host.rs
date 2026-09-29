//! Small host-owned types exposed to machine-local edition extensions.

use std::path::PathBuf;

pub(crate) struct ExtensionContext {
    pub(crate) accounts_path: PathBuf,
    pub(crate) history_dir: PathBuf,
}

pub(crate) struct ExtensionResponse {
    pub(crate) status: &'static str,
    pub(crate) content_type: &'static str,
    pub(crate) body: Vec<u8>,
}

pub(crate) fn log_error(scope: &str, message: &str) {
    crate::log_console_error(scope, message);
}

pub(crate) fn log_event(path: &std::path::Path, scope: &str, message: &str) {
    crate::append_timestamped_log(path, scope, message);
    crate::log_console(scope, message);
}
