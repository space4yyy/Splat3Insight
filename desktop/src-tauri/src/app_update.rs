//! 应用内更新：按 GitHub Releases 里的 latest.json 检查新版本，用户确认后下载、安装并重启。
//!
//! 后台定期检查，发现新版本只提示，不会自动安装：直播时重启会中断 OBS 卡片。

use serde::Serialize;
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::{Update, UpdaterExt};

/// 启动后等一会儿再检查，不和启动时的同步、数据整理抢网络。
const FIRST_CHECK_DELAY: Duration = Duration::from_secs(30);
const CHECK_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

/// 最近一次检查到的新版本，安装时直接用。
#[derive(Default)]
pub(crate) struct PendingUpdate(Mutex<Option<Update>>);

#[derive(Debug, Clone, Serialize)]
pub(crate) struct UpdateInfo {
    version: String,
    notes: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct UpdateProgress {
    downloaded: u64,
    total: Option<u64>,
}

async fn check(app: &AppHandle) -> Result<Option<UpdateInfo>, String> {
    let update = app
        .updater()
        .map_err(|error| format!("检查更新失败：{error}"))?
        .check()
        .await
        .map_err(|error| format!("检查更新失败：{error}"))?;
    let info = update.as_ref().map(|update| UpdateInfo {
        version: update.version.clone(),
        notes: update.body.clone(),
    });
    if let Ok(mut pending) = app.state::<PendingUpdate>().0.lock() {
        *pending = update;
    }
    Ok(info)
}

fn take_pending(app: &AppHandle) -> Option<Update> {
    app.state::<PendingUpdate>()
        .0
        .lock()
        .ok()
        .and_then(|mut pending| pending.take())
}

#[tauri::command]
pub(crate) fn get_app_version(app: AppHandle) -> String {
    app.package_info().version.to_string()
}

#[tauri::command]
pub(crate) async fn check_for_update(app: AppHandle) -> Result<Option<UpdateInfo>, String> {
    check(&app).await
}

/// 下载并安装新版本，然后重启。Windows 上安装程序启动后应用会自行退出。
#[tauri::command]
pub(crate) async fn install_update(app: AppHandle) -> Result<(), String> {
    let update = match take_pending(&app) {
        Some(update) => update,
        None => {
            check(&app).await?;
            take_pending(&app).ok_or_else(|| "已经是最新版本".to_owned())?
        }
    };
    let progress_app = app.clone();
    let mut downloaded = 0_u64;
    update
        .download_and_install(
            |chunk, total| {
                downloaded += chunk as u64;
                let _ = progress_app.emit("update-progress", UpdateProgress { downloaded, total });
            },
            || {},
        )
        .await
        .map_err(|error| format!("安装更新失败：{error}"))?;
    app.restart();
}

/// 后台定期检查，发现新版本时通知窗口。开发版不检查。
pub(crate) fn start_background_checks(app: AppHandle) {
    if cfg!(debug_assertions) {
        return;
    }
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_CHECK_DELAY).await;
        loop {
            match check(&app).await {
                Ok(Some(info)) => {
                    if let Err(error) = app.emit("update-available", info) {
                        crate::log_console_error("update", &format!("通知新版本失败：{error}"));
                    }
                }
                Ok(None) => {}
                Err(error) => crate::log_console_error("update", &error),
            }
            tokio::time::sleep(CHECK_INTERVAL).await;
        }
    });
}
