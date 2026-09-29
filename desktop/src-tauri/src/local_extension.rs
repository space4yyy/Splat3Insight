//! No-op adapter for optional, machine-local build extensions.

#[cfg(feature = "local-extension")]
compile_error!("Replace this no-op adapter with the local extension before enabling the feature.");

use crate::extension_host::{ExtensionContext, ExtensionResponse};
use std::path::Path;

pub(crate) fn route_request(
    _method: &str,
    _path: &str,
    _config: &ExtensionContext,
) -> Result<Option<ExtensionResponse>, String> {
    Ok(None)
}

pub(crate) fn on_history_synced(
    _history_dir: &Path,
    _auth_log_path: &Path,
    _account_id: &str,
    _display_name: &str,
) -> Result<(), String> {
    Ok(())
}
