use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::Local;
use serde::{Deserialize, Serialize};
use splat3_widget_core::{
    history::{HistoryError, HistoryErrorKind, HistorySyncSummary},
    nintendo_auth::{
        begin_login, exchange_session_token, refresh_identity, LoginAttempt, NintendoIdentityError,
    },
    overlay::OverlayState,
    splatnet_auth::{exchange_splatnet_tokens, SplatNetAuthConfig, NXAPI_F_GENERATION_URL},
    AccountMetadata, AccountRegistry, AccountSecret, CachedSecretStore, KeyringSecretStore,
    SecretStore,
};
#[cfg(not(target_os = "macos"))]
use tauri::PhysicalSize;
#[cfg(target_os = "macos")]
use tauri::RunEvent;
use tauri::{
    menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem},
    tray::TrayIconBuilder,
    AppHandle, Emitter, LogicalSize, Manager, Monitor, PhysicalPosition, State, WebviewWindow,
};
use tauri_plugin_autostart::ManagerExt as AutostartManagerExt;
use tokio::sync::watch;

pub(crate) mod history_store;
pub(crate) mod overlay_server;
mod sync_orchestrator;

type Registry = AccountRegistry<CachedSecretStore<KeyringSecretStore>>;

const AUTOSTART_APP_NAME: &str = "Splat3Insight";

struct AppState {
    registry: Arc<Mutex<Registry>>,
    login_attempt: Mutex<Option<LoginAttempt>>,
    auth_log_path: PathBuf,
    history_dir: PathBuf,
    preferences_path: PathBuf,
    last_window_size: Mutex<(u32, u32)>,
    sync_runtime: Arc<SyncRuntime>,
    sync_trigger: Arc<tokio::sync::Notify>,
    overlay_service: overlay_server::OverlayServiceController,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct AppPreferences {
    keep_dock_icon: bool,
    window_placement: Option<SavedWindowPlacement>,
}

impl Default for AppPreferences {
    fn default() -> Self {
        Self {
            keep_dock_icon: true,
            window_placement: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct SavedDisplay {
    name: Option<String>,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct SavedWindowPlacement {
    display: Option<SavedDisplay>,
    offset_x: f64,
    offset_y: f64,
    width: f64,
    height: f64,
}

fn load_app_preferences(path: &Path) -> Result<AppPreferences, String> {
    match fs::read(path) {
        Ok(bytes) => {
            serde_json::from_slice(&bytes).map_err(|error| format!("读取应用设置失败：{error}"))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(AppPreferences::default()),
        Err(error) => Err(format!("读取应用设置失败：{error}")),
    }
}

fn store_app_preferences(path: &Path, preferences: &AppPreferences) -> Result<(), String> {
    let parent = path.parent().ok_or_else(|| "应用设置路径无效".to_owned())?;
    fs::create_dir_all(parent).map_err(|error| format!("创建应用设置目录失败：{error}"))?;

    let temporary_path = path.with_extension(format!("json.{}.tmp", std::process::id()));
    let bytes = serde_json::to_vec_pretty(preferences)
        .map_err(|error| format!("序列化应用设置失败：{error}"))?;
    fs::write(&temporary_path, bytes).map_err(|error| format!("写入应用设置失败：{error}"))?;
    if let Err(error) = fs::rename(&temporary_path, path) {
        let _ = fs::remove_file(&temporary_path);
        return Err(format!("保存应用设置失败：{error}"));
    }

    Ok(())
}

#[cfg(any(target_os = "macos", test))]
fn should_show_main_window_on_reopen(has_visible_windows: bool) -> bool {
    !has_visible_windows
}

const DEFAULT_WINDOW_WIDTH: f64 = 560.0;
const DEFAULT_WINDOW_HEIGHT: f64 = 680.0;
const WINDOW_ASPECT_RATIO: f64 = DEFAULT_WINDOW_WIDTH / DEFAULT_WINDOW_HEIGHT;

fn saved_display_from_monitor(monitor: &Monitor) -> SavedDisplay {
    SavedDisplay {
        name: monitor.name().cloned(),
        x: monitor.position().x,
        y: monitor.position().y,
        width: monitor.size().width,
        height: monitor.size().height,
    }
}

fn display_match_score(saved: &SavedDisplay, candidate: &SavedDisplay) -> Option<(i64, u64)> {
    if saved.name.is_some() && saved.name != candidate.name {
        return None;
    }

    let position_distance = (i64::from(saved.x) - i64::from(candidate.x)).abs()
        + (i64::from(saved.y) - i64::from(candidate.y)).abs();
    let resolution_distance = u64::from(saved.width.abs_diff(candidate.width))
        + u64::from(saved.height.abs_diff(candidate.height));
    Some((position_distance, resolution_distance))
}

fn choose_saved_display(saved: &SavedDisplay, candidates: &[SavedDisplay]) -> Option<usize> {
    candidates
        .iter()
        .enumerate()
        .filter_map(|(index, candidate)| {
            display_match_score(saved, candidate).map(|score| (index, score))
        })
        .min_by_key(|(_, score)| *score)
        .map(|(index, _)| index)
}

fn capture_window_placement(window: &WebviewWindow) -> Result<SavedWindowPlacement, String> {
    let position = window
        .outer_position()
        .map_err(|error| format!("读取窗口位置失败：{error}"))?;
    let size = window
        .inner_size()
        .map_err(|error| format!("读取窗口尺寸失败：{error}"))?;
    let monitor = window
        .current_monitor()
        .map_err(|error| format!("读取当前显示器失败：{error}"))?;
    let scale_factor = monitor
        .as_ref()
        .map(Monitor::scale_factor)
        .unwrap_or_else(|| window.scale_factor().unwrap_or(1.0))
        .max(0.1);
    let (origin_x, origin_y, display) = monitor
        .as_ref()
        .map(|monitor| {
            (
                monitor.position().x,
                monitor.position().y,
                Some(saved_display_from_monitor(monitor)),
            )
        })
        .unwrap_or((0, 0, None));

    Ok(SavedWindowPlacement {
        display,
        offset_x: f64::from(position.x - origin_x) / scale_factor,
        offset_y: f64::from(position.y - origin_y) / scale_factor,
        width: f64::from(size.width) / scale_factor,
        height: f64::from(size.height) / scale_factor,
    })
}

fn persist_window_placement(window: &WebviewWindow, path: &Path) -> Result<(), String> {
    let placement = capture_window_placement(window)?;
    let mut preferences = load_app_preferences(path)?;
    preferences.window_placement = Some(placement);
    store_app_preferences(path, &preferences)
}

fn restore_window_placement(
    window: &WebviewWindow,
    placement: Option<&SavedWindowPlacement>,
) -> Result<(), String> {
    let Some(placement) = placement else {
        return Ok(());
    };
    let monitors = window
        .available_monitors()
        .map_err(|error| format!("读取显示器列表失败：{error}"))?;
    if monitors.is_empty() {
        return Ok(());
    }

    let target_monitor = placement
        .display
        .as_ref()
        .and_then(|saved| {
            let candidates: Vec<_> = monitors.iter().map(saved_display_from_monitor).collect();
            choose_saved_display(saved, &candidates).map(|index| monitors[index].clone())
        })
        .or(window
            .primary_monitor()
            .map_err(|error| format!("读取主显示器失败：{error}"))?)
        .or_else(|| monitors.first().cloned());
    let Some(monitor) = target_monitor else {
        return Ok(());
    };

    let scale_factor = monitor.scale_factor().max(0.1);
    let work_area = monitor.work_area();
    let work_width = f64::from(work_area.size.width) / scale_factor;
    let work_height = f64::from(work_area.size.height) / scale_factor;
    let width = placement
        .width
        .min(work_width)
        .min(work_height * WINDOW_ASPECT_RATIO);
    let height = width / WINDOW_ASPECT_RATIO;

    let old_scale_factor = window.scale_factor().unwrap_or(scale_factor).max(0.1);
    let old_inner_size = window
        .inner_size()
        .map_err(|error| format!("读取窗口尺寸失败：{error}"))?;
    let old_outer_size = window
        .outer_size()
        .map_err(|error| format!("读取窗口边框尺寸失败：{error}"))?;
    let frame_width =
        f64::from(old_outer_size.width.saturating_sub(old_inner_size.width)) / old_scale_factor;
    let frame_height =
        f64::from(old_outer_size.height.saturating_sub(old_inner_size.height)) / old_scale_factor;
    let outer_width = (width + frame_width) * scale_factor;
    let outer_height = (height + frame_height) * scale_factor;

    window
        .set_size(LogicalSize::new(width, height))
        .map_err(|error| format!("恢复窗口尺寸失败：{error}"))?;

    let monitor_position = monitor.position();
    let area_position = work_area.position;
    let x = clamp_window_axis(
        f64::from(monitor_position.x) + placement.offset_x * scale_factor,
        f64::from(area_position.x),
        f64::from(work_area.size.width),
        outer_width,
    );
    let y = clamp_window_axis(
        f64::from(monitor_position.y) + placement.offset_y * scale_factor,
        f64::from(area_position.y),
        f64::from(work_area.size.height),
        outer_height,
    );
    window
        .set_position(PhysicalPosition::new(x, y))
        .map_err(|error| format!("恢复窗口位置失败：{error}"))
}

fn clamp_window_axis(position: f64, area_start: f64, area_size: f64, window_size: f64) -> i32 {
    let max_position = (area_start + area_size - window_size).max(area_start);
    position.clamp(area_start, max_position).round() as i32
}

#[cfg(target_os = "macos")]
fn set_window_content_aspect_ratio(window: &WebviewWindow) -> Result<(), String> {
    let pointer = window
        .ns_window()
        .map_err(|error| format!("读取 macOS 窗口句柄失败：{error}"))?;
    let native_window = unsafe { &*pointer.cast::<objc2_app_kit::NSWindow>() };
    native_window.setContentAspectRatio(objc2_foundation::NSSize::new(
        DEFAULT_WINDOW_WIDTH,
        DEFAULT_WINDOW_HEIGHT,
    ));
    Ok(())
}

#[cfg(any(not(target_os = "macos"), test))]
fn aspect_corrected_size(previous: (u32, u32), requested: (u32, u32)) -> (u32, u32) {
    let (width, height) = requested;
    if width == 0 || height == 0 {
        return previous;
    }
    if (f64::from(width) / f64::from(height) - WINDOW_ASPECT_RATIO).abs() < 0.002 {
        return requested;
    }

    let width_change = width.abs_diff(previous.0) as f64;
    let height_change_as_width = height.abs_diff(previous.1) as f64 * WINDOW_ASPECT_RATIO;
    if width_change >= height_change_as_width {
        (
            width,
            (f64::from(width) / WINDOW_ASPECT_RATIO).round().max(1.0) as u32,
        )
    } else {
        (
            (f64::from(height) * WINDOW_ASPECT_RATIO).round().max(1.0) as u32,
            height,
        )
    }
}

#[derive(Debug, Clone, Serialize)]
struct SyncStatus {
    state: String,
    account_id: Option<String>,
    message: String,
    phase: String,
    progress_completed: usize,
    progress_total: usize,
    retry_attempt: usize,
    initial: bool,
    added_battle_count: usize,
    added_coop_count: usize,
    battle_count: usize,
    coop_count: usize,
    started_at_ms: Option<i64>,
    completed_at_ms: Option<i64>,
    error: Option<String>,
}

struct SyncRuntime {
    statuses: Mutex<HashMap<String, SyncStatus>>,
    tasks: Mutex<HashMap<String, watch::Sender<Option<SyncResult>>>>,
}

type SyncResult = Result<HistorySyncSummary, String>;

#[derive(Debug, Clone)]
enum SyncError {
    TokenExpired(String),
    Other(String),
}

impl SyncError {
    fn from_history(kind: HistoryErrorKind, message: String) -> Self {
        match kind {
            HistoryErrorKind::TokenExpired => Self::TokenExpired(message),
            HistoryErrorKind::Other => Self::Other(message),
        }
    }

    fn is_token_expired(&self) -> bool {
        matches!(self, Self::TokenExpired(_))
    }
}

impl std::fmt::Display for SyncError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TokenExpired(message) | Self::Other(message) => formatter.write_str(message),
        }
    }
}

impl From<String> for SyncError {
    fn from(message: String) -> Self {
        Self::Other(message)
    }
}

impl From<&str> for SyncError {
    fn from(message: &str) -> Self {
        Self::Other(message.to_owned())
    }
}

impl From<HistoryError> for SyncError {
    fn from(error: HistoryError) -> Self {
        Self::from_history(error.kind(), error.to_string())
    }
}

enum SyncClaim {
    Owner(watch::Sender<Option<SyncResult>>),
    Join(watch::Receiver<Option<SyncResult>>),
}

impl SyncRuntime {
    fn new() -> Self {
        Self {
            statuses: Mutex::new(HashMap::new()),
            tasks: Mutex::new(HashMap::new()),
        }
    }

    fn claim_or_join(&self, account_id: &str) -> Result<SyncClaim, String> {
        let mut tasks = self.tasks.lock().map_err(|_| "sync state is locked")?;
        if let Some(sender) = tasks.get(account_id) {
            return Ok(SyncClaim::Join(sender.subscribe()));
        }

        let (sender, _receiver) = watch::channel(None);
        tasks.insert(account_id.to_owned(), sender.clone());
        Ok(SyncClaim::Owner(sender))
    }

    fn complete(
        &self,
        account_id: &str,
        sender: &watch::Sender<Option<SyncResult>>,
        result: SyncResult,
    ) {
        sender.send_replace(Some(result));
        let mut tasks = self
            .tasks
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        tasks.remove(account_id);
    }

    fn is_in_progress(&self, account_id: &str) -> Result<bool, String> {
        Ok(self
            .tasks
            .lock()
            .map_err(|_| "sync state is locked")?
            .contains_key(account_id))
    }

    fn update(&self, account_id: &str, update: impl FnOnce(&mut SyncStatus)) {
        if let Ok(mut statuses) = self.statuses.lock() {
            let status = statuses
                .entry(account_id.to_owned())
                .or_insert_with(SyncStatus::default);
            status.account_id = Some(account_id.to_owned());
            update(status);
        }
    }

    fn get(&self, account_id: Option<&str>) -> SyncStatus {
        let Some(account_id) = account_id else {
            return SyncStatus {
                message: "等待登录账号".to_owned(),
                ..SyncStatus::default()
            };
        };
        self.statuses
            .lock()
            .ok()
            .and_then(|statuses| statuses.get(account_id).cloned())
            .unwrap_or_else(|| SyncStatus {
                account_id: Some(account_id.to_owned()),
                message: "等待同步".to_owned(),
                ..SyncStatus::default()
            })
    }
}

struct SyncOwnerGuard {
    runtime: Arc<SyncRuntime>,
    account_id: String,
    sender: Option<watch::Sender<Option<SyncResult>>>,
}

impl SyncOwnerGuard {
    fn complete(mut self, result: SyncResult) {
        if let Some(sender) = self.sender.take() {
            self.runtime.complete(&self.account_id, &sender, result);
        }
    }
}

impl Drop for SyncOwnerGuard {
    fn drop(&mut self) {
        if let Some(sender) = self.sender.take() {
            let message = "同步任务提前结束，请重试".to_owned();
            self.runtime.update(&self.account_id, |status| {
                status.state = "error".to_owned();
                status.phase = "error".to_owned();
                status.message = "同步任务提前结束".to_owned();
                status.error = Some(message.clone());
                status.completed_at_ms = Some(now_ms());
            });
            self.runtime
                .complete(&self.account_id, &sender, Err(message));
        }
    }
}

impl Default for SyncStatus {
    fn default() -> Self {
        Self {
            state: "idle".to_owned(),
            account_id: None,
            message: "等待同步".to_owned(),
            phase: "idle".to_owned(),
            progress_completed: 0,
            progress_total: 0,
            retry_attempt: 0,
            initial: false,
            added_battle_count: 0,
            added_coop_count: 0,
            battle_count: 0,
            coop_count: 0,
            started_at_ms: None,
            completed_at_ms: None,
            error: None,
        }
    }
}

const HISTORY_WINDOW_SIZE: usize = 50;
const BACKGROUND_SYNC_INTERVAL_SECS: u64 = 15;
const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;
const LOG_BACKUP_COUNT: usize = 3;

#[derive(Serialize)]
struct AccountView {
    account_id: String,
    display_name: String,
    language: Option<String>,
    country: Option<String>,
    active: bool,
    splatnet_ready: Option<bool>,
    history_battle_count: usize,
    history_coop_count: usize,
    history_fetched_at_ms: Option<i64>,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock is before Unix epoch")
        .as_millis() as i64
}

pub(crate) fn local_log_timestamp() -> String {
    Local::now().format("%Y-%m-%d %H:%M:%S%.3f %:z").to_string()
}

fn write_auth_log(path: &Path, stage: &str, message: &str) {
    let safe_message = message.replace(['\n', '\r'], " ");
    append_timestamped_log(path, stage, &safe_message);
    log_console(&format!("splat3-auth:{stage}"), &safe_message);
}

fn write_account_log(
    path: &Path,
    stage: &str,
    display_name: &str,
    account_id: &str,
    message: &str,
) {
    write_auth_log(
        path,
        stage,
        &format!("账号“{display_name}”（ID: {account_id}）：{message}"),
    );
}

fn account_display_name(registry: &Arc<Mutex<Registry>>, account_id: &str) -> String {
    registry
        .lock()
        .ok()
        .and_then(|registry| {
            registry
                .accounts()
                .iter()
                .find(|account| account.account_id == account_id)
                .map(|account| account.display_name.clone())
        })
        .unwrap_or_else(|| "未知账号".to_owned())
}

fn account_auth_error(
    path: &Path,
    stage: &str,
    display_name: &str,
    account_id: &str,
    error: String,
) -> String {
    write_account_log(path, stage, display_name, account_id, &error);
    format!("{error}\n认证日志：{}", path.display())
}

pub(crate) fn append_timestamped_log(path: &Path, scope: &str, message: &str) {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    rotate_log_if_needed(path);
    let safe_message = message.replace(['\n', '\r'], " ");
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(
            file,
            "{} [{}] {}",
            local_log_timestamp(),
            scope,
            safe_message
        );
    }
}

fn rotate_log_if_needed(path: &Path) {
    let Ok(metadata) = fs::metadata(path) else {
        return;
    };
    if metadata.len() < MAX_LOG_BYTES {
        return;
    }
    let oldest = PathBuf::from(format!("{}.{}", path.display(), LOG_BACKUP_COUNT));
    let _ = fs::remove_file(oldest);
    for index in (1..LOG_BACKUP_COUNT).rev() {
        let source = PathBuf::from(format!("{}.{}", path.display(), index));
        let target = PathBuf::from(format!("{}.{}", path.display(), index + 1));
        let _ = fs::rename(source, target);
    }
    let first_backup = PathBuf::from(format!("{}.1", path.display()));
    let _ = fs::rename(path, first_backup);
}

pub(crate) fn log_console(scope: &str, message: &str) {
    let safe_message = message.replace(['\n', '\r'], " ");
    println!("{} [{}] {}", local_log_timestamp(), scope, safe_message);
}

pub(crate) fn log_console_error(scope: &str, message: &str) {
    let safe_message = message.replace(['\n', '\r'], " ");
    eprintln!("{} [{}] {}", local_log_timestamp(), scope, safe_message);
}

fn auth_error(path: &Path, stage: &str, error: String) -> String {
    write_auth_log(path, stage, &error);
    format!("{error}\n认证日志：{}", path.display())
}

fn load_history_summary(
    history_dir: &Path,
    account_id: &str,
) -> Result<Option<HistorySyncSummary>, String> {
    history_store::load_summary(history_dir, account_id)
}

fn empty_overlay_state() -> OverlayState {
    OverlayState {
        selected: None,
        recent_summary: splat3_widget_core::overlay::RecentSummary {
            sample_size: 0,
            win: 0,
            lose: 0,
            win_rate: 0.0,
        },
    }
}

fn update_sync_status(
    runtime: &SyncRuntime,
    account_id: &str,
    update: impl FnOnce(&mut SyncStatus),
) {
    runtime.update(account_id, update);
}

fn active_account_sync_state<S: SecretStore>(
    registry: &Arc<Mutex<AccountRegistry<S>>>,
) -> Result<Option<(String, bool)>, String> {
    let mut registry = registry.lock().map_err(|_| "account registry is locked")?;
    let Some(account) = registry.active_account().cloned() else {
        return Ok(None);
    };
    let account_id = account.account_id;
    if registry.is_account_deletion_pending(&account_id) {
        return Ok(None);
    }
    let ready = ensure_account_readiness(&mut registry, &account_id)?;
    Ok(Some((account_id, ready)))
}

fn ensure_account_readiness<S: SecretStore>(
    registry: &mut AccountRegistry<S>,
    account_id: &str,
) -> Result<bool, String> {
    if let Some(ready) = registry
        .accounts()
        .iter()
        .find(|account| account.account_id == account_id)
        .and_then(|account| account.splatnet_ready)
    {
        return Ok(ready);
    }

    let secret = match registry.get_secret(account_id) {
        Ok(secret) => secret,
        Err(error) => {
            // Don't retry a denied/locked Keychain lookup on every poll. The
            // account can retry explicitly through its permission action.
            registry.set_splatnet_ready(account_id, false)?;
            return Err(error);
        }
    };
    let ready = secret
        .is_some_and(|secret| secret.game_web_token.is_some() && secret.bullet_token.is_some());
    registry.set_splatnet_ready(account_id, ready)?;
    Ok(ready)
}

async fn sync_account(
    account_id: String,
    registry: Arc<Mutex<Registry>>,
    history_dir: PathBuf,
    auth_log_path: PathBuf,
    sync_runtime: Arc<SyncRuntime>,
) -> Result<HistorySyncSummary, String> {
    let owner = match sync_runtime.claim_or_join(&account_id)? {
        SyncClaim::Owner(sender) => SyncOwnerGuard {
            runtime: Arc::clone(&sync_runtime),
            account_id: account_id.clone(),
            sender: Some(sender),
        },
        SyncClaim::Join(mut receiver) => loop {
            if let Some(result) = receiver.borrow().clone() {
                return result;
            }
            if receiver.changed().await.is_err() {
                return Err("同步任务提前结束，请重试".to_owned());
            }
        },
    };
    let display_name = account_display_name(&registry, &account_id);

    let initial_result = sync_orchestrator::sync_account_inner(
        &account_id,
        &registry,
        &history_dir,
        &auth_log_path,
        &sync_runtime,
    )
    .await;

    let result = match initial_result {
        Err(error) if error.is_token_expired() => {
            update_sync_status(&sync_runtime, &account_id, |status| {
                status.state = "refreshing".to_owned();
                status.account_id = Some(account_id.clone());
                status.message = "SplatNet token 已失效，正在自动刷新…".to_owned();
                status.phase = "refreshing".to_owned();
                status.progress_completed = 0;
                status.progress_total = 0;
                status.retry_attempt = 0;
                status.error = None;
            });
            write_account_log(
                &auth_log_path,
                "token-refresh",
                &display_name,
                &account_id,
                "检测到 SplatNet token 失效，尝试使用已保存的 Nintendo session token 自动刷新",
            );
            match refresh_splatnet_tokens_for_account(&account_id, &registry, &auth_log_path).await
            {
                Ok(()) => {
                    write_account_log(
                        &auth_log_path,
                        "token-refresh",
                        &display_name,
                        &account_id,
                        "SplatNet token 自动刷新成功，重试历史同步",
                    );
                    sync_orchestrator::sync_account_inner(
                        &account_id,
                        &registry,
                        &history_dir,
                        &auth_log_path,
                        &sync_runtime,
                    )
                    .await
                    .map_err(|error| error.to_string())
                }
                Err(refresh_error) => Err(format_token_refresh_error(&refresh_error)),
            }
        }
        Ok(summary) => Ok(summary),
        Err(error) => Err(error.to_string()),
    };

    match &result {
        Ok(summary) => update_sync_status(&sync_runtime, &account_id, |status| {
            status.state = "completed".to_owned();
            status.phase = "completed".to_owned();
            status.progress_completed = 0;
            status.progress_total = 0;
            status.retry_attempt = 0;
            status.message = format!(
                "同步完成：新增 {} 条对战、{} 条打工",
                summary.added_battle_count, summary.added_coop_count
            );
            status.added_battle_count = summary.added_battle_count;
            status.added_coop_count = summary.added_coop_count;
            status.battle_count = summary.battle_count;
            status.coop_count = summary.coop_count;
            status.completed_at_ms = Some(now_ms());
            status.error = None;
        }),
        Err(error) => update_sync_status(&sync_runtime, &account_id, |status| {
            status.state = "error".to_owned();
            status.phase = "error".to_owned();
            status.retry_attempt = 0;
            status.message = "同步失败".to_owned();
            status.error = Some(error.clone());
            status.completed_at_ms = Some(now_ms());
        }),
    }
    owner.complete(result.clone());
    result
}

#[derive(Debug, PartialEq, Eq)]
enum TokenRefreshFailure {
    NintendoSessionExpired,
    Failed(String),
}

impl From<String> for TokenRefreshFailure {
    fn from(error: String) -> Self {
        Self::Failed(error)
    }
}

impl From<&str> for TokenRefreshFailure {
    fn from(error: &str) -> Self {
        Self::Failed(error.to_owned())
    }
}

fn format_token_refresh_error(error: &TokenRefreshFailure) -> String {
    match error {
        TokenRefreshFailure::NintendoSessionExpired => {
            "Nintendo 登录状态已失效，请重新添加账号。".to_owned()
        }
        TokenRefreshFailure::Failed(details) => {
            format!("SplatNet token 自动刷新失败，请检查网络或稍后重试：{details}")
        }
    }
}

fn start_background_sync(
    registry: Arc<Mutex<Registry>>,
    history_dir: PathBuf,
    auth_log_path: PathBuf,
    sync_runtime: Arc<SyncRuntime>,
    sync_trigger: Arc<tokio::sync::Notify>,
) {
    tauri::async_runtime::spawn(async move {
        // Let the main webview paint before the first Keychain lookup can
        // display a macOS authorization prompt.
        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        loop {
            let next_sync_at = tokio::time::Instant::now()
                + std::time::Duration::from_secs(BACKGROUND_SYNC_INTERVAL_SECS);
            match active_account_sync_state(&registry) {
                Ok(Some((account_id, true))) => {
                    let _ = sync_account(
                        account_id,
                        Arc::clone(&registry),
                        history_dir.clone(),
                        auth_log_path.clone(),
                        Arc::clone(&sync_runtime),
                    )
                    .await;
                }
                Ok(Some((account_id, false))) => {
                    let status_account_id = account_id.clone();
                    update_sync_status(&sync_runtime, &account_id, |status| {
                        status.state = "idle".to_owned();
                        status.account_id = Some(status_account_id);
                        status.message = "等待获取 SplatNet 权限".to_owned();
                        status.phase = "idle".to_owned();
                        status.progress_completed = 0;
                        status.progress_total = 0;
                        status.retry_attempt = 0;
                        status.error = None;
                    });
                }
                Ok(None) => {}
                Err(error) => {
                    if let Ok(registry) = registry.lock() {
                        if let Some(account_id) = registry.active_account_id() {
                            update_sync_status(&sync_runtime, account_id, |status| {
                                status.state = "error".to_owned();
                                status.message = "读取活动账号失败".to_owned();
                                status.phase = "error".to_owned();
                                status.progress_completed = 0;
                                status.progress_total = 0;
                                status.retry_attempt = 0;
                                status.error = Some(error);
                            });
                        }
                    }
                }
            }
            let _ = tokio::time::timeout(
                next_sync_at.saturating_duration_since(tokio::time::Instant::now()),
                sync_trigger.notified(),
            )
            .await;
        }
    });
}

fn build_account_views<S: SecretStore>(
    registry: &mut AccountRegistry<S>,
    history_dir: &Path,
) -> Result<Vec<AccountView>, String> {
    let active_id = registry.active_account_id();
    registry
        .accounts()
        .iter()
        .map(|account| {
            let history = load_history_summary(history_dir, &account.account_id)?;
            Ok(AccountView {
                account_id: account.account_id.clone(),
                display_name: account.display_name.clone(),
                language: account.language.clone(),
                country: account.country.clone(),
                active: active_id == Some(account.account_id.as_str()),
                splatnet_ready: account.splatnet_ready,
                history_battle_count: history.as_ref().map_or(0, |value| value.battle_count),
                history_coop_count: history.as_ref().map_or(0, |value| value.coop_count),
                history_fetched_at_ms: history.map(|value| value.fetched_at_ms),
            })
        })
        .collect::<Result<Vec<_>, String>>()
}

#[tauri::command]
fn list_accounts(state: State<'_, AppState>) -> Result<Vec<AccountView>, String> {
    let mut registry = state
        .registry
        .lock()
        .map_err(|_| "account registry is locked")?;
    build_account_views(&mut registry, &state.history_dir)
}

#[tauri::command]
fn get_sync_status(state: State<'_, AppState>) -> Result<SyncStatus, String> {
    let account_id = state
        .registry
        .lock()
        .map_err(|_| "account registry is locked")?
        .active_account_id()
        .map(ToOwned::to_owned);
    Ok(state.sync_runtime.get(account_id.as_deref()))
}

fn resolve_account_id(
    registry: &Arc<Mutex<Registry>>,
    requested_account_id: Option<String>,
) -> Result<String, String> {
    let registry = registry.lock().map_err(|_| "account registry is locked")?;
    let account_id = requested_account_id
        .or_else(|| registry.active_account_id().map(ToOwned::to_owned))
        .ok_or_else(|| "还没有选择活动账号".to_owned())?;
    if !registry
        .accounts()
        .iter()
        .any(|account| account.account_id == account_id)
    {
        return Err(format!("unknown account: {account_id}"));
    }
    if registry.is_account_deletion_pending(&account_id) {
        return Err("该账号正在完成删除，请稍后重试".to_owned());
    }
    Ok(account_id)
}

#[tauri::command]
fn export_history(
    account_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let account_id = resolve_account_id(&state.registry, account_id)?;
    let path = history_store::export(&state.history_dir, &account_id)?;
    let display_name = account_display_name(&state.registry, &account_id);
    write_account_log(
        &state.auth_log_path,
        "history-export",
        &display_name,
        &account_id,
        &format!("历史数据已导出：{}", path.display()),
    );
    Ok(path.display().to_string())
}

#[tauri::command]
fn remove_account(account_id: String, state: State<'_, AppState>) -> Result<(), String> {
    let account_id = account_id.trim().to_owned();
    if account_id.is_empty() {
        return Err("账号 ID 不能为空".to_owned());
    }

    let mut registry = state
        .registry
        .lock()
        .map_err(|_| "account registry is locked")?;
    if !registry
        .accounts()
        .iter()
        .any(|account| account.account_id == account_id)
    {
        return Err(format!("unknown account: {account_id}"));
    }
    if state.sync_runtime.is_in_progress(&account_id)? {
        return Err("该账号正在同步，请等待同步完成后再删除".to_owned());
    }

    let display_name = registry
        .accounts()
        .iter()
        .find(|account| account.account_id == account_id)
        .map(|account| account.display_name.clone())
        .unwrap_or_else(|| "未知账号".to_owned());
    registry.mark_account_deletion_pending(&account_id)?;
    history_store::delete_account(&state.history_dir, &account_id)?;
    registry.remove(&account_id)?;
    registry.clear_account_deletion_pending(&account_id)?;
    write_account_log(
        &state.auth_log_path,
        "account-delete",
        &display_name,
        &account_id,
        "账号及本地历史数据已删除",
    );
    state.sync_trigger.notify_one();
    Ok(())
}

#[tauri::command]
fn get_overlay_service_status(
    state: State<'_, AppState>,
) -> Result<overlay_server::OverlayServiceStatus, String> {
    state
        .overlay_service
        .status()
        .lock()
        .map(|status| status.clone())
        .map_err(|_| "OBS service status is locked".to_owned())
}

#[tauri::command]
fn start_overlay_service(
    state: State<'_, AppState>,
) -> Result<overlay_server::OverlayServiceStatus, String> {
    state.overlay_service.start()?;
    get_overlay_service_status(state)
}

#[tauri::command]
fn stop_overlay_service(
    state: State<'_, AppState>,
) -> Result<overlay_server::OverlayServiceStatus, String> {
    state.overlay_service.stop()?;
    get_overlay_service_status(state)
}

#[tauri::command]
fn get_overlay_state(
    account_id: Option<String>,
    state: State<'_, AppState>,
) -> Result<OverlayState, String> {
    let account_id = {
        let registry = state
            .registry
            .lock()
            .map_err(|_| "account registry is locked")?;
        let account_id = account_id
            .or_else(|| registry.active_account_id().map(ToOwned::to_owned))
            .ok_or_else(|| "还没有选择活动账号".to_owned())?;
        if !registry
            .accounts()
            .iter()
            .any(|account| account.account_id == account_id)
        {
            return Err(format!("unknown account: {account_id}"));
        }
        if registry.is_account_deletion_pending(&account_id) {
            return Ok(empty_overlay_state());
        }
        account_id
    };

    history_store::load_overlay_state(&state.history_dir, &account_id)
}

#[tauri::command]
fn begin_nintendo_login(state: State<'_, AppState>) -> Result<String, String> {
    let attempt = begin_login()?;
    let authorize_url = attempt.authorize_url.clone();
    *state
        .login_attempt
        .lock()
        .map_err(|_| "login state is locked")? = Some(attempt);
    write_auth_log(
        &state.auth_log_path,
        "nintendo-login",
        "开始添加 Nintendo Account，已生成授权链接",
    );
    Ok(authorize_url)
}

#[tauri::command]
fn cancel_nintendo_login(state: State<'_, AppState>) -> Result<(), String> {
    *state
        .login_attempt
        .lock()
        .map_err(|_| "login state is locked")? = None;
    write_auth_log(
        &state.auth_log_path,
        "nintendo-login",
        "已取消 Nintendo Account 添加流程",
    );
    Ok(())
}

#[tauri::command]
async fn complete_nintendo_login(
    callback_url: String,
    state: State<'_, AppState>,
) -> Result<String, String> {
    let auth_log_path = state.auth_log_path.clone();
    write_auth_log(&auth_log_path, "start", "开始 Nintendo Account 认证");
    let attempt = state
        .login_attempt
        .lock()
        .map_err(|_| "login state is locked")?
        .take()
        .ok_or_else(|| {
            auth_error(
                &auth_log_path,
                "input",
                "no Nintendo login is pending".to_owned(),
            )
        })?;
    let callback = attempt
        .finish(&callback_url)
        .map_err(|error| auth_error(&auth_log_path, "nintendo-link", error))?;
    write_auth_log(&auth_log_path, "nintendo-link", "已解析选择此人链接");
    let identity = exchange_session_token(callback)
        .await
        .map_err(|error| auth_error(&auth_log_path, "nintendo-session", error))?;
    let account_id = identity.account_id.clone();
    let display_name = identity.display_name.clone();
    write_account_log(
        &auth_log_path,
        "nintendo-session",
        &display_name,
        &account_id,
        "Nintendo session token 交换成功",
    );
    let timestamp = now_ms();

    let metadata = AccountMetadata {
        account_id: account_id.clone(),
        display_name: display_name.clone(),
        language: identity.language,
        country: identity.country,
        splatnet_ready: Some(false),
        created_at_ms: timestamp,
        last_used_at_ms: timestamp,
    };
    let secret = AccountSecret {
        session_token: identity.session_token,
        game_web_token: None,
        bullet_token: None,
    };

    let mut registry = state
        .registry
        .lock()
        .map_err(|_| "account registry is locked")?;
    registry.upsert(metadata, secret)?;
    write_account_log(
        &auth_log_path,
        "nintendo-session",
        &display_name,
        &account_id,
        "Nintendo Account 已保存，应用将继续获取 SplatNet token",
    );
    registry.set_active(&account_id, timestamp)?;
    drop(registry);
    write_account_log(
        &auth_log_path,
        "account-switch",
        &display_name,
        &account_id,
        "新账号已设为当前账号",
    );
    state.sync_trigger.notify_one();
    Ok(account_id)
}

async fn refresh_splatnet_tokens_for_account(
    account_id: &str,
    registry: &Arc<Mutex<Registry>>,
    auth_log_path: &Path,
) -> Result<(), TokenRefreshFailure> {
    let (metadata, session_token) = {
        let registry = registry.lock().map_err(|_| "account registry is locked")?;
        let metadata = registry
            .accounts()
            .iter()
            .find(|account| account.account_id == account_id)
            .cloned()
            .ok_or_else(|| format!("unknown account: {account_id}"))?;
        let secret = registry
            .get_secret(account_id)?
            .ok_or_else(|| "该账号没有保存 Nintendo session token".to_owned())?;
        (metadata, secret.session_token)
    };

    write_account_log(
        auth_log_path,
        "splatnet-refresh",
        &metadata.display_name,
        account_id,
        "使用已保存的 Nintendo session token 刷新 SplatNet token",
    );
    let identity = refresh_identity(&session_token).await.map_err(|error| {
        let details = account_auth_error(
            auth_log_path,
            "nintendo-session-refresh",
            &metadata.display_name,
            account_id,
            error.to_string(),
        );
        match error {
            NintendoIdentityError::SessionTokenRejected => {
                TokenRefreshFailure::NintendoSessionExpired
            }
            _ => TokenRefreshFailure::Failed(details),
        }
    })?;
    let auth_config = SplatNetAuthConfig::default();
    write_account_log(
        auth_log_path,
        "f-config",
        &metadata.display_name,
        account_id,
        &format!("使用 nxapi f 参数服务：{NXAPI_F_GENERATION_URL}"),
    );
    let splatnet_tokens = exchange_splatnet_tokens(&identity, &auth_config)
        .await
        .map_err(|error| {
            account_auth_error(
                auth_log_path,
                "splatnet-tokens",
                &metadata.display_name,
                account_id,
                error,
            )
        })?;
    let secret = AccountSecret {
        session_token,
        game_web_token: Some(splatnet_tokens.game_web_token),
        bullet_token: Some(splatnet_tokens.bullet_token),
    };
    let mut metadata = metadata;
    metadata.display_name = identity.display_name;
    metadata.language = identity.language;
    metadata.country = identity.country;
    metadata.splatnet_ready = Some(true);
    let display_name = metadata.display_name.clone();

    let mut registry = registry.lock().map_err(|_| "account registry is locked")?;
    registry.upsert(metadata, secret)?;
    write_account_log(
        auth_log_path,
        "splatnet-tokens",
        &display_name,
        account_id,
        "game web token 和 bullet token 获取成功",
    );
    Ok(())
}

#[tauri::command]
async fn refresh_splatnet_tokens(
    account_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    refresh_splatnet_tokens_for_account(&account_id, &state.registry, &state.auth_log_path)
        .await
        .map_err(|error| format_token_refresh_error(&error))?;
    state.sync_trigger.notify_one();
    Ok(())
}

#[tauri::command]
async fn sync_history(
    account_id: String,
    state: State<'_, AppState>,
) -> Result<HistorySyncSummary, String> {
    sync_account(
        account_id,
        Arc::clone(&state.registry),
        state.history_dir.clone(),
        state.auth_log_path.clone(),
        Arc::clone(&state.sync_runtime),
    )
    .await
}

#[tauri::command]
fn set_active_account(account_id: String, state: State<'_, AppState>) -> Result<(), String> {
    let mut registry = state
        .registry
        .lock()
        .map_err(|_| "account registry is locked")?;
    let target = registry
        .accounts()
        .iter()
        .find(|account| account.account_id == account_id)
        .cloned()
        .ok_or_else(|| format!("unknown account: {account_id}"))?;
    let previous = registry.active_account().cloned();
    registry.set_active(&account_id, now_ms())?;
    drop(registry);
    let previous_label = previous
        .as_ref()
        .map(|account| {
            format!(
                "账号“{}”（ID: {}）",
                account.display_name, account.account_id
            )
        })
        .unwrap_or_else(|| "无".to_owned());
    write_auth_log(
        &state.auth_log_path,
        "account-switch",
        &format!(
            "切换当前账号：{previous_label} -> 账号“{}”（ID: {}）",
            target.display_name, target.account_id
        ),
    );
    state.sync_trigger.notify_one();
    Ok(())
}

#[tauri::command]
fn hide_main_window(window: WebviewWindow) -> Result<(), String> {
    save_main_window_placement(window.app_handle());
    window.hide().map_err(|error| error.to_string())
}

#[tauri::command]
fn exit_app(app: AppHandle) {
    save_main_window_placement(&app);
    app.exit(0);
}

fn save_main_window_placement(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let path = app.state::<AppState>().preferences_path.clone();
    if let Err(error) = persist_window_placement(&window, &path) {
        log_console_error("app-window", &error);
    }
}

fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

#[cfg(target_os = "macos")]
fn associate_autostart_agent_with_app(app: &AppHandle) -> Result<(), String> {
    let executable_path = std::env::current_exe().map_err(|error| error.to_string())?;
    let agent_path = app
        .path()
        .home_dir()
        .map_err(|error| error.to_string())?
        .join("Library/LaunchAgents")
        .join(format!("{AUTOSTART_APP_NAME}.plist"));
    associate_launch_agent_with_bundle(&agent_path, &app.config().identifier, &executable_path)
}

#[cfg(target_os = "macos")]
fn associate_launch_agent_with_bundle(
    agent_path: &Path,
    bundle_identifier: &str,
    executable_path: &Path,
) -> Result<(), String> {
    if !agent_path.exists() {
        return Ok(());
    }

    let mut launch_agent = plist::Value::from_file(&agent_path)
        .map_err(|error| format!("读取 macOS 登录项配置失败：{error}"))?;
    let dictionary = launch_agent
        .as_dictionary_mut()
        .ok_or_else(|| "macOS 登录项配置格式无效".to_owned())?;
    let arguments = dictionary
        .get_mut("ProgramArguments")
        .and_then(plist::Value::as_array_mut)
        .ok_or_else(|| "macOS 登录项缺少启动命令".to_owned())?;
    match arguments.first_mut() {
        Some(plist::Value::String(executable)) => {
            *executable = executable_path.display().to_string();
        }
        _ => return Err("macOS 登录项启动命令格式无效".to_owned()),
    }
    dictionary.insert(
        "AssociatedBundleIdentifiers".to_owned(),
        plist::Value::Array(vec![plist::Value::String(bundle_identifier.to_owned())]),
    );

    let mut serialized = Vec::new();
    plist::to_writer_xml(&mut serialized, &launch_agent)
        .map_err(|error| format!("更新 macOS 登录项配置失败：{error}"))?;
    let temporary_path = agent_path.with_extension(format!("plist.{}.tmp", std::process::id()));
    fs::write(&temporary_path, serialized)
        .map_err(|error| format!("写入 macOS 登录项配置失败：{error}"))?;
    if let Err(error) = fs::rename(&temporary_path, &agent_path) {
        let _ = fs::remove_file(&temporary_path);
        return Err(format!("保存 macOS 登录项配置失败：{error}"));
    }

    Ok(())
}

fn toggle_autostart(app: &AppHandle, menu_item: &CheckMenuItem<tauri::Wry>) {
    let manager = app.autolaunch();
    let was_enabled = manager.is_enabled().unwrap_or(false);
    let result = if was_enabled {
        manager.disable()
    } else {
        manager.enable().and_then(|()| {
            #[cfg(target_os = "macos")]
            if let Err(error) = associate_autostart_agent_with_app(app) {
                let _ = manager.disable();
                return Err(tauri_plugin_autostart::Error::Anyhow(error));
            }
            Ok(())
        })
    };

    match result {
        Ok(()) => {
            let _ = menu_item.set_checked(!was_enabled);
            log_console(
                "app-lifecycle",
                if was_enabled {
                    "已关闭开机启动"
                } else {
                    "已开启开机启动"
                },
            );
        }
        Err(error) => {
            let _ = menu_item.set_checked(was_enabled);
            log_console_error("app-lifecycle", &format!("设置开机启动失败：{error}"));
        }
    }
}

#[cfg(target_os = "macos")]
fn toggle_keep_dock_icon(
    app: &AppHandle,
    menu_item: &CheckMenuItem<tauri::Wry>,
    preferences_path: &Path,
) {
    // Check menu items are toggled by the native menu before the event arrives.
    let enabled = menu_item.is_checked().unwrap_or(true);
    let previous_value = !enabled;
    if let Err(error) = app.set_dock_visibility(enabled) {
        let _ = menu_item.set_checked(previous_value);
        log_console_error("app-lifecycle", &format!("设置程序坞图标失败：{error}"));
        return;
    }

    let mut preferences = match load_app_preferences(preferences_path) {
        Ok(preferences) => preferences,
        Err(error) => {
            let _ = app.set_dock_visibility(previous_value);
            let _ = menu_item.set_checked(previous_value);
            log_console_error("app-lifecycle", &error);
            return;
        }
    };
    preferences.keep_dock_icon = enabled;
    if let Err(error) = store_app_preferences(preferences_path, &preferences) {
        let _ = app.set_dock_visibility(previous_value);
        let _ = menu_item.set_checked(previous_value);
        log_console_error("app-lifecycle", &error);
        return;
    }

    log_console(
        "app-lifecycle",
        if enabled {
            "已保留程序坞图标"
        } else {
            "已隐藏程序坞图标；可从菜单栏图标重新打开主界面"
        },
    );
}

fn build_tray(
    app: &mut tauri::App,
    preferences_path: PathBuf,
    preferences: &AppPreferences,
) -> tauri::Result<()> {
    #[cfg(not(target_os = "macos"))]
    let _ = (&preferences_path, preferences);

    let autostart_enabled = app.autolaunch().is_enabled().unwrap_or(false);
    let open_item = MenuItem::with_id(app, "open-main", "打开主界面", true, None::<&str>)?;
    let autostart_item = CheckMenuItem::with_id(
        app,
        "toggle-autostart",
        "开机启动",
        true,
        autostart_enabled,
        None::<&str>,
    )?;
    let separator = PredefinedMenuItem::separator(app)?;
    let exit_item = MenuItem::with_id(app, "exit-app", "退出", true, None::<&str>)?;

    #[cfg(target_os = "macos")]
    let (menu, dock_icon_item) = {
        let dock_icon_item = CheckMenuItem::with_id(
            app,
            "toggle-dock-icon",
            "保留程序坞图标",
            true,
            preferences.keep_dock_icon,
            None::<&str>,
        )?;
        let menu = Menu::with_items(
            app,
            &[
                &open_item,
                &autostart_item,
                &dock_icon_item,
                &separator,
                &exit_item,
            ],
        )?;
        (menu, dock_icon_item)
    };

    #[cfg(not(target_os = "macos"))]
    let menu = Menu::with_items(app, &[&open_item, &autostart_item, &separator, &exit_item])?;

    let autostart_item_for_events = autostart_item.clone();
    #[cfg(target_os = "macos")]
    let dock_icon_item_for_events = dock_icon_item.clone();

    TrayIconBuilder::with_id("main-tray")
        .icon(tauri::include_image!("./icons/44x44.png"))
        .icon_as_template(false)
        .tooltip("斯普拉遁3直播小组件")
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(move |app, event| match event.id().as_ref() {
            "open-main" => show_main_window(app),
            "toggle-autostart" => toggle_autostart(app, &autostart_item_for_events),
            #[cfg(target_os = "macos")]
            "toggle-dock-icon" => {
                toggle_keep_dock_icon(app, &dock_icon_item_for_events, &preferences_path)
            }
            "exit-app" => app.exit(0),
            _ => {}
        })
        .build(app)?;

    Ok(())
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(
            tauri_plugin_autostart::Builder::new()
                .app_name(AUTOSTART_APP_NAME)
                .args(["--background"])
                .build(),
        )
        .setup(|app| {
            let launch_in_background =
                std::env::args_os().any(|argument| argument == "--background");
            #[cfg(target_os = "macos")]
            if app.autolaunch().is_enabled().unwrap_or(false) {
                if let Err(error) = associate_autostart_agent_with_app(app.handle()) {
                    log_console_error(
                        "app-lifecycle",
                        &format!("修复 macOS 登录项标识失败：{error}"),
                    );
                }
            }
            let config_dir = app.path().app_config_dir()?;
            let preferences_path = config_dir.join("preferences.json");
            let preferences = match load_app_preferences(&preferences_path) {
                Ok(preferences) => preferences,
                Err(error) => {
                    log_console_error("app-lifecycle", &error);
                    AppPreferences::default()
                }
            };
            #[cfg(target_os = "macos")]
            if let Err(error) = app.handle().set_dock_visibility(preferences.keep_dock_icon) {
                log_console_error("app-lifecycle", &format!("设置程序坞图标失败：{error}"));
            }

            let main_window = app
                .get_webview_window("main")
                .ok_or_else(|| std::io::Error::other("主窗口未创建"))?;
            #[cfg(target_os = "macos")]
            if let Err(error) = set_window_content_aspect_ratio(&main_window) {
                log_console_error("app-window", &error);
            }
            let initial_window_size = main_window
                .inner_size()
                .map(|size| (size.width, size.height))
                .unwrap_or((DEFAULT_WINDOW_WIDTH as u32, DEFAULT_WINDOW_HEIGHT as u32));

            let metadata_path = config_dir.join("accounts.json");
            let history_dir = config_dir.join("history");
            let mut registry = AccountRegistry::load(
                &metadata_path,
                CachedSecretStore::new(KeyringSecretStore::default()),
            )
            .map_err(std::io::Error::other)?;
            for account_id in registry.pending_account_deletions().to_vec() {
                let display_name = registry
                    .accounts()
                    .iter()
                    .find(|account| account.account_id == account_id)
                    .map(|account| account.display_name.clone())
                    .unwrap_or_else(|| "未知账号".to_owned());
                let cleanup = history_store::delete_account(&history_dir, &account_id)
                    .and_then(|()| registry.remove(&account_id))
                    .and_then(|()| registry.clear_account_deletion_pending(&account_id));
                if let Err(error) = cleanup {
                    log_console_error(
                        "account-delete-recovery",
                        &format!("账号“{display_name}”删除未完成：{error}"),
                    );
                } else {
                    log_console(
                        "account-delete-recovery",
                        &format!("账号“{display_name}”上次未完成的删除已完成"),
                    );
                }
            }
            let registry = Arc::new(Mutex::new(registry));
            let sync_runtime = Arc::new(SyncRuntime::new());
            let sync_trigger = Arc::new(tokio::sync::Notify::new());
            if let Err(error) = history_store::check_integrity(&history_dir) {
                log_console_error("history", &error);
            }
            let overlay_service = overlay_server::OverlayServiceController::new(
                overlay_server::OverlayServerConfig {
                    accounts_path: metadata_path,
                    history_dir: history_dir.clone(),
                },
            );
            app.manage(AppState {
                registry: Arc::clone(&registry),
                login_attempt: Mutex::new(None),
                auth_log_path: config_dir.join("auth.log"),
                history_dir: history_dir.clone(),
                preferences_path: preferences_path.clone(),
                last_window_size: Mutex::new(initial_window_size),
                sync_runtime: Arc::clone(&sync_runtime),
                sync_trigger: Arc::clone(&sync_trigger),
                overlay_service: overlay_service.clone(),
            });
            if let Err(error) =
                restore_window_placement(&main_window, preferences.window_placement.as_ref())
            {
                log_console_error("app-window", &error);
            }
            if let Ok(size) = main_window.inner_size() {
                *app.state::<AppState>()
                    .last_window_size
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = (size.width, size.height);
            }

            overlay_service.start().map_err(std::io::Error::other)?;

            build_tray(app, preferences_path, &preferences)?;
            if !launch_in_background {
                main_window.show()?;
            }
            start_background_sync(
                registry,
                history_dir,
                config_dir.join("auth.log"),
                sync_runtime,
                sync_trigger,
            );

            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::Resized(size) = event {
                let app_state = window.app_handle().state::<AppState>();
                #[cfg(not(target_os = "macos"))]
                {
                    let requested = (size.width, size.height);
                    let mut last_size = app_state
                        .last_window_size
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    let corrected = aspect_corrected_size(*last_size, requested);
                    *last_size = corrected;
                    if corrected != requested {
                        if let Err(error) =
                            window.set_size(PhysicalSize::new(corrected.0, corrected.1))
                        {
                            log_console_error("app-window", &format!("保持窗口比例失败：{error}"));
                        }
                    }
                }
                #[cfg(target_os = "macos")]
                {
                    *app_state
                        .last_window_size
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner) =
                        (size.width, size.height);
                }
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                save_main_window_placement(window.app_handle());
                api.prevent_close();
                if let Err(error) = window.emit("main-close-requested", ()) {
                    log_console_error(
                        "app-lifecycle",
                        &format!("通知前端处理窗口关闭失败：{error}"),
                    );
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            list_accounts,
            get_sync_status,
            export_history,
            remove_account,
            get_overlay_service_status,
            start_overlay_service,
            stop_overlay_service,
            get_overlay_state,
            begin_nintendo_login,
            cancel_nintendo_login,
            complete_nintendo_login,
            refresh_splatnet_tokens,
            sync_history,
            set_active_account,
            hide_main_window,
            exit_app,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Splat3 Insight")
        .run(|app, event| match event {
            #[cfg(target_os = "macos")]
            RunEvent::Reopen {
                has_visible_windows,
                ..
            } if should_show_main_window_on_reopen(has_visible_windows) => {
                show_main_window(app);
            }
            RunEvent::ExitRequested { .. } => save_main_window_placement(app),
            _ => {}
        });
}

#[cfg(test)]
mod tests {
    use super::{
        active_account_sync_state, append_timestamped_log, aspect_corrected_size,
        build_account_views, choose_saved_display, format_token_refresh_error,
        load_app_preferences, local_log_timestamp, should_show_main_window_on_reopen,
        store_app_preferences, AppPreferences, HistoryErrorKind, HistorySyncSummary, SavedDisplay,
        SavedWindowPlacement, SyncClaim, SyncError, SyncRuntime, TokenRefreshFailure,
        LOG_BACKUP_COUNT, MAX_LOG_BYTES,
    };
    use splat3_widget_core::{
        AccountMetadata, AccountRegistry, AccountSecret, CachedSecretStore, SecretStore,
    };
    use std::fs;
    #[cfg(target_os = "macos")]
    use std::path::Path;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use tempfile::tempdir;

    #[test]
    fn dock_reopen_shows_hidden_main_window() {
        assert!(should_show_main_window_on_reopen(false));
        assert!(!should_show_main_window_on_reopen(true));
    }

    #[test]
    fn dock_icon_preference_defaults_to_visible_and_persists_changes() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("preferences.json");
        let placement = SavedWindowPlacement {
            display: Some(SavedDisplay {
                name: Some("Display A".to_owned()),
                x: -1920,
                y: 0,
                width: 1920,
                height: 1080,
            }),
            offset_x: 120.0,
            offset_y: 80.0,
            width: 560.0,
            height: 680.0,
        };

        assert!(load_app_preferences(&path).unwrap().keep_dock_icon);
        store_app_preferences(
            &path,
            &AppPreferences {
                keep_dock_icon: false,
                window_placement: Some(placement.clone()),
            },
        )
        .unwrap();
        let saved = load_app_preferences(&path).unwrap();
        assert!(!saved.keep_dock_icon);
        assert_eq!(saved.window_placement, Some(placement));
    }

    #[test]
    fn saved_window_restores_to_the_matching_display() {
        let saved = SavedDisplay {
            name: Some("External Display".to_owned()),
            x: -1920,
            y: 0,
            width: 1920,
            height: 1080,
        };
        let displays = vec![
            SavedDisplay {
                name: Some("Built-in Display".to_owned()),
                x: 0,
                y: 0,
                width: 3024,
                height: 1964,
            },
            saved.clone(),
            SavedDisplay {
                name: Some("External Display".to_owned()),
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
            },
        ];

        assert_eq!(choose_saved_display(&saved, &displays), Some(1));
    }

    #[test]
    fn proportional_resize_tracks_horizontal_or_vertical_drag() {
        assert_eq!(aspect_corrected_size((560, 680), (700, 680)), (700, 850));
        assert_eq!(aspect_corrected_size((560, 680), (560, 800)), (659, 800));
    }

    #[derive(Clone)]
    struct ReadCountingStore {
        get_count: Arc<AtomicUsize>,
        secret: Option<AccountSecret>,
        error: Option<String>,
    }

    impl SecretStore for ReadCountingStore {
        fn put(&self, _account_id: &str, _secret: &AccountSecret) -> Result<(), String> {
            Ok(())
        }

        fn get(&self, _account_id: &str) -> Result<Option<AccountSecret>, String> {
            self.get_count.fetch_add(1, Ordering::Relaxed);
            match &self.error {
                Some(error) => Err(error.clone()),
                None => Ok(self.secret.clone()),
            }
        }

        fn delete(&self, _account_id: &str) -> Result<(), String> {
            Ok(())
        }
    }

    #[test]
    fn listing_accounts_does_not_read_any_credentials() {
        let directory = tempdir().unwrap();
        let read_count = Arc::new(AtomicUsize::new(0));
        let store = CachedSecretStore::new(ReadCountingStore {
            get_count: Arc::clone(&read_count),
            secret: None,
            error: None,
        });
        let mut registry =
            AccountRegistry::load(directory.path().join("accounts.json"), store).unwrap();
        for (id, name) in [("a", "Alice"), ("b", "Bob")] {
            registry
                .upsert(
                    AccountMetadata {
                        account_id: id.to_owned(),
                        display_name: name.to_owned(),
                        language: None,
                        country: None,
                        splatnet_ready: Some(true),
                        created_at_ms: 1,
                        last_used_at_ms: 1,
                    },
                    AccountSecret {
                        session_token: "test-session".to_owned(),
                        game_web_token: Some("test-web".to_owned()),
                        bullet_token: Some("test-bullet".to_owned()),
                    },
                )
                .unwrap();
        }
        registry.set_active("a", 1).unwrap();

        let views = build_account_views(&mut registry, directory.path()).unwrap();

        assert_eq!(views.len(), 2);
        assert!(views
            .iter()
            .all(|account| account.splatnet_ready == Some(true)));
        assert_eq!(read_count.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn listing_legacy_accounts_does_not_read_credentials() {
        let directory = tempdir().unwrap();
        let read_count = Arc::new(AtomicUsize::new(0));
        let store = ReadCountingStore {
            get_count: Arc::clone(&read_count),
            secret: Some(AccountSecret {
                session_token: "test-session".to_owned(),
                game_web_token: Some("test-web".to_owned()),
                bullet_token: Some("test-bullet".to_owned()),
            }),
            error: None,
        };
        let metadata_path = directory.path().join("accounts.json");
        fs::write(
            &metadata_path,
            r#"{
                "active_account_id":"a",
                "accounts":[
                    {"account_id":"a","display_name":"Alice","created_at_ms":1,"last_used_at_ms":1},
                    {"account_id":"b","display_name":"Bob","created_at_ms":1,"last_used_at_ms":1}
                ]
            }"#,
        )
        .unwrap();
        let mut registry = AccountRegistry::load(&metadata_path, store).unwrap();

        let views = build_account_views(&mut registry, directory.path()).unwrap();

        assert_eq!(read_count.load(Ordering::Relaxed), 0);
        assert_eq!(views[0].splatnet_ready, None);
        assert_eq!(views[1].splatnet_ready, None);
    }

    #[test]
    fn active_account_migrates_legacy_readiness_once() {
        let directory = tempdir().unwrap();
        let read_count = Arc::new(AtomicUsize::new(0));
        let store = CachedSecretStore::new(ReadCountingStore {
            get_count: Arc::clone(&read_count),
            secret: Some(AccountSecret {
                session_token: "test-session".to_owned(),
                game_web_token: Some("test-web".to_owned()),
                bullet_token: Some("test-bullet".to_owned()),
            }),
            error: None,
        });
        let metadata_path = directory.path().join("accounts.json");
        fs::write(
            &metadata_path,
            r#"{
                "active_account_id":"a",
                "accounts":[{
                    "account_id":"a",
                    "display_name":"Alice",
                    "language":null,
                    "country":null,
                    "created_at_ms":1,
                    "last_used_at_ms":1
                }]
            }"#,
        )
        .unwrap();
        let registry = AccountRegistry::load(&metadata_path, store).unwrap();
        let registry = Arc::new(std::sync::Mutex::new(registry));

        assert_eq!(
            active_account_sync_state(&registry).unwrap(),
            Some(("a".to_owned(), true))
        );
        assert_eq!(
            active_account_sync_state(&registry).unwrap(),
            Some(("a".to_owned(), true))
        );
        assert_eq!(read_count.load(Ordering::Relaxed), 1);
        assert_eq!(
            registry.lock().unwrap().accounts()[0].splatnet_ready,
            Some(true)
        );
    }

    #[test]
    fn denied_legacy_keychain_lookup_is_not_retried_on_each_poll() {
        let directory = tempdir().unwrap();
        let read_count = Arc::new(AtomicUsize::new(0));
        let store = ReadCountingStore {
            get_count: Arc::clone(&read_count),
            secret: None,
            error: Some("keychain access denied".to_owned()),
        };
        let metadata_path = directory.path().join("accounts.json");
        fs::write(
            &metadata_path,
            r#"{
                "active_account_id":"a",
                "accounts":[{"account_id":"a","display_name":"Alice","created_at_ms":1,"last_used_at_ms":1}]
            }"#,
        )
        .unwrap();
        let registry = AccountRegistry::load(&metadata_path, store).unwrap();
        let registry = Arc::new(std::sync::Mutex::new(registry));

        assert_eq!(
            active_account_sync_state(&registry).unwrap_err(),
            "keychain access denied"
        );
        assert_eq!(
            active_account_sync_state(&registry).unwrap(),
            Some(("a".to_owned(), false))
        );
        assert_eq!(read_count.load(Ordering::Relaxed), 1);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn associates_login_agent_with_app_bundle_identifier() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("Splat3Insight.plist");
        let mut dictionary = plist::Dictionary::new();
        dictionary.insert(
            "Label".to_owned(),
            plist::Value::String("Splat3Insight".to_owned()),
        );
        dictionary.insert(
            "ProgramArguments".to_owned(),
            plist::Value::Array(vec![
                plist::Value::String("/old/path/LegacyExecutable".to_owned()),
                plist::Value::String("--background".to_owned()),
            ]),
        );
        plist::Value::Dictionary(dictionary)
            .to_file_xml(&path)
            .unwrap();

        let expected_executable =
            Path::new("/Applications/Splat3 Insight.app/Contents/MacOS/Splat3Insight");
        super::associate_launch_agent_with_bundle(
            &path,
            "ink.space4.Splat3Insight",
            expected_executable,
        )
        .unwrap();

        let updated = plist::Value::from_file(path).unwrap();
        let dictionary = updated.as_dictionary().unwrap();
        assert_eq!(
            dictionary.get("AssociatedBundleIdentifiers"),
            Some(&plist::Value::Array(vec![plist::Value::String(
                "ink.space4.Splat3Insight".to_owned()
            )]))
        );
        assert_eq!(
            dictionary
                .get("ProgramArguments")
                .and_then(plist::Value::as_array)
                .and_then(|arguments| arguments.first())
                .and_then(plist::Value::as_string),
            expected_executable.to_str()
        );
        assert_eq!(
            dictionary
                .get("ProgramArguments")
                .and_then(plist::Value::as_array)
                .and_then(|arguments| arguments.get(1))
                .and_then(plist::Value::as_string),
            Some("--background")
        );
    }

    #[test]
    fn refreshes_only_for_typed_splatnet_token_expiration_errors() {
        assert!(SyncError::from_history(
            HistoryErrorKind::TokenExpired,
            "对战列表查询失败：HTTP 401".to_owned()
        )
        .is_token_expired());
        assert!(!SyncError::from_history(
            HistoryErrorKind::Other,
            "SplatNet 数据请求失败（HTTP 401）".to_owned()
        )
        .is_token_expired());
        assert!(!SyncError::from_history(
            HistoryErrorKind::Other,
            "SplatNet 数据请求失败（HTTP 429）".to_owned()
        )
        .is_token_expired());
    }

    #[test]
    fn transient_nintendo_refresh_failures_do_not_request_a_new_login() {
        let message = format_token_refresh_error(&TokenRefreshFailure::Failed(
            "连接 Nintendo 账号服务失败，请检查网络后重试".to_owned(),
        ));
        assert!(message.contains("稍后重试"));
        assert!(!message.contains("请重新登录"));
    }

    #[test]
    fn rejected_nintendo_session_token_requests_a_new_login() {
        let message = format_token_refresh_error(&TokenRefreshFailure::NintendoSessionExpired);
        assert!(message.contains("请重新添加账号"));
    }

    #[test]
    fn local_log_timestamp_contains_timezone_offset() {
        let timestamp = local_log_timestamp();
        assert_eq!(timestamp.len(), 30);
        assert!(matches!(timestamp.as_bytes()[24], b'+' | b'-'));
        assert_eq!(timestamp.as_bytes()[27], b':');
    }

    #[test]
    fn sync_runtime_joins_duplicate_account_requests() {
        let runtime = SyncRuntime::new();
        let SyncClaim::Owner(owner) = runtime.claim_or_join("account-a").unwrap() else {
            panic!("first request should own the sync");
        };
        let SyncClaim::Owner(other_account) = runtime.claim_or_join("account-b").unwrap() else {
            panic!("different accounts should sync independently");
        };
        let SyncClaim::Join(joiner) = runtime.claim_or_join("account-a").unwrap() else {
            panic!("duplicate request should join the existing sync");
        };

        let result = HistorySyncSummary {
            battle_count: 4,
            coop_count: 2,
            fetched_at_ms: 123,
            added_battle_count: 1,
            added_coop_count: 0,
        };
        runtime.complete("account-a", &owner, Ok(result.clone()));

        assert_eq!(joiner.borrow().as_ref(), Some(&Ok(result)));
        assert!(matches!(
            runtime.claim_or_join("account-a").unwrap(),
            SyncClaim::Owner(_)
        ));
        runtime.complete("account-b", &other_account, Err("test complete".to_owned()));
        drop(other_account);
    }

    #[test]
    fn rotates_logs_into_bounded_backups() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("app.log");
        fs::write(&path, vec![b'x'; MAX_LOG_BYTES as usize]).unwrap();
        for index in 1..=LOG_BACKUP_COUNT {
            fs::write(
                format!("{}.{}", path.display(), index),
                format!("backup-{index}"),
            )
            .unwrap();
        }

        append_timestamped_log(&path, "test", "rotated");

        assert!(path.exists());
        assert_eq!(
            fs::read_to_string(format!("{}.1", path.display()))
                .unwrap()
                .len(),
            MAX_LOG_BYTES as usize
        );
        assert_eq!(
            fs::read_to_string(format!("{}.2", path.display())).unwrap(),
            "backup-1"
        );
        assert_eq!(
            fs::read_to_string(format!("{}.3", path.display())).unwrap(),
            "backup-2"
        );
        assert!(!path.with_extension("log.4").exists());
    }
}
