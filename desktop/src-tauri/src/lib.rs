use std::collections::HashMap;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::Local;
use serde::Serialize;
use splat3_widget_core::{
    history::{
        HistoryError, HistoryErrorKind, HistoryFetchProgress, HistoryFetchedRecord,
        HistoryProgressCallback, HistoryRecordCallback, HistorySyncSummary, SplatNetDataClient,
        SplatNetDataConfig,
    },
    nintendo_auth::{begin_login, exchange_session_token, refresh_identity, LoginAttempt},
    overlay::OverlayState,
    splatnet_auth::{exchange_splatnet_tokens, SplatNetAuthConfig, NXAPI_F_GENERATION_URL},
    AccountMetadata, AccountRegistry, AccountSecret, KeyringSecretStore,
};
use tauri::{Manager, State};
use tokio::sync::watch;

pub(crate) mod history_store;
mod overlay_server;

type Registry = AccountRegistry<KeyringSecretStore>;

struct AppState {
    registry: Arc<Mutex<Registry>>,
    login_attempt: Mutex<Option<LoginAttempt>>,
    auth_log_path: PathBuf,
    history_dir: PathBuf,
    sync_runtime: Arc<SyncRuntime>,
    sync_trigger: Arc<tokio::sync::Notify>,
    overlay_service: overlay_server::OverlayServiceController,
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
    splatnet_ready: bool,
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

fn active_account_sync_state(
    registry: &Arc<Mutex<Registry>>,
) -> Result<Option<(String, bool)>, String> {
    let registry = registry.lock().map_err(|_| "account registry is locked")?;
    let Some(account_id) = registry.active_account_id().map(ToOwned::to_owned) else {
        return Ok(None);
    };
    if registry.is_account_deletion_pending(&account_id) {
        return Ok(None);
    }
    let ready = registry
        .get_secret(&account_id)?
        .is_some_and(|secret| secret.game_web_token.is_some() && secret.bullet_token.is_some());
    Ok(Some((account_id, ready)))
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

    let initial_result = sync_account_inner(
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
                    sync_account_inner(
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

fn format_token_refresh_error(error: &str) -> String {
    if error.contains("session token")
        || error.contains("Nintendo")
        || error.contains("Nintendo Account")
        || error.contains("重新登录")
    {
        return format!("SplatNet token 已失效，Nintendo 登录凭据也已失效，请重新登录：{error}");
    }
    format!("SplatNet token 已失效，自动刷新失败，请稍后重试：{error}")
}

async fn sync_account_inner(
    account_id: &str,
    registry: &Arc<Mutex<Registry>>,
    history_dir: &Path,
    auth_log_path: &Path,
    sync_runtime: &Arc<SyncRuntime>,
) -> Result<HistorySyncSummary, SyncError> {
    let (metadata, secret) = {
        let registry = registry.lock().map_err(|_| "account registry is locked")?;
        let metadata = registry
            .accounts()
            .iter()
            .find(|account| account.account_id == account_id)
            .cloned()
            .ok_or_else(|| format!("unknown account: {account_id}"))?;
        if registry.is_account_deletion_pending(account_id) {
            return Err(SyncError::Other(
                "该账号正在完成删除，请稍后重试".to_owned(),
            ));
        }
        let secret = registry
            .get_secret(account_id)?
            .ok_or_else(|| "该账号没有保存登录信息".to_owned())?;
        (metadata, secret)
    };
    let game_web_token = secret
        .game_web_token
        .ok_or_else(|| "请先获取 SplatNet 权限，再同步历史记录".to_owned())?;
    let bullet_token = secret
        .bullet_token
        .ok_or_else(|| "请先获取 SplatNet 权限，再同步历史记录".to_owned())?;
    let config = SplatNetDataConfig::new(
        game_web_token,
        bullet_token,
        metadata.language.as_deref().unwrap_or("en-US"),
        metadata.country.as_deref().unwrap_or("US"),
    );

    update_sync_status(sync_runtime, account_id, |status| {
        status.state = "preparing".to_owned();
        status.account_id = Some(account_id.to_owned());
        status.message = "正在准备 SplatNet 数据客户端…".to_owned();
        status.phase = "preparing".to_owned();
        status.progress_completed = 0;
        status.progress_total = 0;
        status.retry_attempt = 0;
        status.started_at_ms = Some(now_ms());
        status.completed_at_ms = None;
        status.error = None;
    });

    let existing_summary = load_history_summary(history_dir, account_id).map_err(|error| {
        account_auth_error(
            auth_log_path,
            "history-summary",
            &metadata.display_name,
            account_id,
            error,
        )
    })?;
    let initial_sync =
        history_store::needs_full_sync(history_dir, account_id).map_err(|error| {
            account_auth_error(
                auth_log_path,
                "history-state",
                &metadata.display_name,
                account_id,
                error,
            )
        })?;
    let (known_battle_ids, known_coop_ids) =
        history_store::load_known_record_ids(history_dir, account_id).map_err(|error| {
            account_auth_error(
                auth_log_path,
                "history-dedup",
                &metadata.display_name,
                account_id,
                error,
            )
        })?;

    let sync_description = if initial_sync {
        "首次同步：正在拉取当前账号可用的全部历史对战和打工记录…"
    } else {
        "增量同步：正在检查最近记录中的新数据…"
    };
    update_sync_status(sync_runtime, account_id, |status| {
        status.state = "syncing".to_owned();
        status.account_id = Some(account_id.to_owned());
        status.message = sync_description.to_owned();
        status.phase = "syncing".to_owned();
        status.progress_completed = 0;
        status.progress_total = 0;
        status.retry_attempt = 0;
        status.initial = initial_sync;
        status.battle_count = existing_summary
            .as_ref()
            .map(|summary| summary.battle_count)
            .unwrap_or_default();
        status.coop_count = existing_summary
            .as_ref()
            .map(|summary| summary.coop_count)
            .unwrap_or_default();
    });
    write_account_log(
        auth_log_path,
        "history-sync",
        &metadata.display_name,
        account_id,
        if initial_sync {
            "开始首次完整同步：拉取当前账号可用的全部对战和打工记录"
        } else {
            "开始增量同步：检查最近 50 条对战和 50 条打工记录"
        },
    );

    let fetched_at_ms = now_ms();
    history_store::mark_sync_started(history_dir, account_id, fetched_at_ms).map_err(|error| {
        account_auth_error(
            auth_log_path,
            "history-state",
            &metadata.display_name,
            account_id,
            error,
        )
    })?;

    let client = SplatNetDataClient::connect(config).await.map_err(|error| {
        account_auth_error(
            auth_log_path,
            "history-client",
            &metadata.display_name,
            account_id,
            error,
        )
    })?;
    let progress_status = Arc::clone(sync_runtime);
    let progress_account_id = account_id.to_owned();
    let progress: HistoryProgressCallback = Arc::new(move |progress: HistoryFetchProgress| {
        update_sync_status(&progress_status, &progress_account_id, |status| {
            status.phase = progress.phase.clone();
            status.progress_completed = progress.completed;
            status.progress_total = progress.total;
            status.retry_attempt = progress.retry_attempt;
            status.message = match progress.phase.as_str() {
                "battle-list" => "正在获取最近对战列表…".to_owned(),
                "coop-list" => "正在获取最近打工列表…".to_owned(),
                "battle-details" => "正在获取对战详情…".to_owned(),
                "coop-details" => "正在获取打工详情…".to_owned(),
                phase if phase.starts_with("retrying:") => "网络波动，正在重试…".to_owned(),
                _ => "正在同步数据…".to_owned(),
            };
        });
    });
    let record_writer = Arc::new(Mutex::new(
        history_store::HistoryRecordWriter::open(history_dir).map_err(|error| {
            account_auth_error(
                auth_log_path,
                "history-store",
                &metadata.display_name,
                account_id,
                error,
            )
        })?,
    ));
    let record_writer_for_callback = Arc::clone(&record_writer);
    let record_account_id = account_id.to_owned();
    let on_record: HistoryRecordCallback = Arc::new(move |record: HistoryFetchedRecord| {
        record_writer_for_callback
            .lock()
            .map_err(|_| "历史记录写入器已锁定".to_owned())?
            .save(&record_account_id, fetched_at_ms, record)
    });
    let cache = if initial_sync {
        client
            .fetch_history_full_with_progress_and_records_from_typed(
                &known_battle_ids,
                &known_coop_ids,
                fetched_at_ms,
                Arc::clone(&progress),
                Some(Arc::clone(&on_record)),
            )
            .await
            .map_err(|error| {
                let kind = error.kind();
                let message = account_auth_error(
                    auth_log_path,
                    "history-fetch",
                    &metadata.display_name,
                    account_id,
                    error.to_string(),
                );
                SyncError::from_history(kind, message)
            })?
    } else {
        client
            .fetch_history_incremental_with_progress_and_records_typed(
                HISTORY_WINDOW_SIZE,
                HISTORY_WINDOW_SIZE,
                &known_battle_ids,
                &known_coop_ids,
                fetched_at_ms,
                Arc::clone(&progress),
                Some(Arc::clone(&on_record)),
            )
            .await
            .map_err(|error| {
                let kind = error.kind();
                let message = account_auth_error(
                    auth_log_path,
                    "history-fetch",
                    &metadata.display_name,
                    account_id,
                    error.to_string(),
                );
                SyncError::from_history(kind, message)
            })?
    };
    drop(on_record);
    drop(record_writer);
    let added_battle_count = cache.battles.len();
    let added_coop_count = cache.coop.len();
    if initial_sync || added_battle_count > 0 {
        if let Some(overlay_cache) = history_store::load_overlay_cache(history_dir, account_id)
            .map_err(|error| {
                account_auth_error(
                    auth_log_path,
                    "overlay-cache",
                    &metadata.display_name,
                    account_id,
                    error,
                )
            })?
        {
            history_store::save_overlay_state(
                history_dir,
                account_id,
                &OverlayState::from_history(&overlay_cache),
            )
            .map_err(|error| {
                account_auth_error(
                    auth_log_path,
                    "overlay-cache",
                    &metadata.display_name,
                    account_id,
                    error,
                )
            })?;
        }
    }
    history_store::mark_sync_complete(history_dir, account_id, fetched_at_ms).map_err(|error| {
        account_auth_error(
            auth_log_path,
            "history-state",
            &metadata.display_name,
            account_id,
            error,
        )
    })?;
    let mut summary = load_history_summary(history_dir, account_id)
        .map_err(|error| {
            account_auth_error(
                auth_log_path,
                "history-summary",
                &metadata.display_name,
                account_id,
                error,
            )
        })?
        .unwrap_or_default();
    summary.added_battle_count = added_battle_count;
    summary.added_coop_count = added_coop_count;
    write_account_log(
        auth_log_path,
        "history-sync",
        &metadata.display_name,
        account_id,
        &format!(
            "历史记录同步成功：新增 {} 条对战、{} 条打工；归档共 {} 条对战、{} 条打工",
            summary.added_battle_count,
            summary.added_coop_count,
            summary.battle_count,
            summary.coop_count
        ),
    );
    Ok(summary)
}

fn start_background_sync(
    registry: Arc<Mutex<Registry>>,
    history_dir: PathBuf,
    auth_log_path: PathBuf,
    sync_runtime: Arc<SyncRuntime>,
    sync_trigger: Arc<tokio::sync::Notify>,
) {
    tauri::async_runtime::spawn(async move {
        loop {
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
                std::time::Duration::from_secs(BACKGROUND_SYNC_INTERVAL_SECS),
                sync_trigger.notified(),
            )
            .await;
        }
    });
}

#[tauri::command]
fn list_accounts(state: State<'_, AppState>) -> Result<Vec<AccountView>, String> {
    let registry = state
        .registry
        .lock()
        .map_err(|_| "account registry is locked")?;
    let active_id = registry.active_account_id();
    registry
        .accounts()
        .iter()
        .map(|account| {
            let secret = registry.get_secret(&account.account_id)?;
            let history = load_history_summary(&state.history_dir, &account.account_id)?;
            Ok(AccountView {
                account_id: account.account_id.clone(),
                display_name: account.display_name.clone(),
                language: account.language.clone(),
                country: account.country.clone(),
                active: active_id == Some(account.account_id.as_str()),
                splatnet_ready: secret.as_ref().is_some_and(|value| {
                    value.game_web_token.is_some() && value.bullet_token.is_some()
                }),
                history_battle_count: history.as_ref().map_or(0, |value| value.battle_count),
                history_coop_count: history.as_ref().map_or(0, |value| value.coop_count),
                history_fetched_at_ms: history.map(|value| value.fetched_at_ms),
            })
        })
        .collect::<Result<Vec<_>, String>>()
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
        created_at_ms: timestamp,
        last_used_at_ms: timestamp,
        baseline_at_ms: None,
        last_seen_battle_id: None,
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
) -> Result<(), String> {
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
        account_auth_error(
            auth_log_path,
            "nintendo-session-refresh",
            &metadata.display_name,
            account_id,
            error,
        )
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
    refresh_splatnet_tokens_for_account(&account_id, &state.registry, &state.auth_log_path).await?;
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

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let config_dir = app.path().app_config_dir()?;
            let metadata_path = config_dir.join("accounts.json");
            let history_dir = config_dir.join("history");
            let mut registry = AccountRegistry::load(&metadata_path, KeyringSecretStore::default())
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
                sync_runtime: Arc::clone(&sync_runtime),
                sync_trigger: Arc::clone(&sync_trigger),
                overlay_service: overlay_service.clone(),
            });

            start_background_sync(
                registry,
                history_dir,
                config_dir.join("auth.log"),
                sync_runtime,
                sync_trigger,
            );

            overlay_service.start().map_err(std::io::Error::other)?;

            Ok(())
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running Splat 3 Streaming Widget");
}

#[cfg(test)]
mod tests {
    use super::{
        append_timestamped_log, local_log_timestamp, HistoryErrorKind, HistorySyncSummary,
        SyncClaim, SyncError, SyncRuntime, LOG_BACKUP_COUNT, MAX_LOG_BYTES,
    };
    use std::fs;
    use tempfile::tempdir;

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
