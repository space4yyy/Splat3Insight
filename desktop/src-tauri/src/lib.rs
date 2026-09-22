use std::collections::HashSet;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use splat3_widget_core::{
    history::{HistoryCache, HistorySyncSummary, SplatNetDataClient, SplatNetDataConfig},
    nintendo_auth::{begin_login, exchange_session_token, refresh_identity, LoginAttempt},
    splatnet_auth::{exchange_splatnet_tokens, SplatNetAuthConfig, NXAPI_F_GENERATION_URL},
    AccountMetadata, AccountRegistry, AccountSecret, KeyringSecretStore,
};
use tauri::{Manager, State};

type Registry = AccountRegistry<KeyringSecretStore>;

struct AppState {
    registry: Mutex<Registry>,
    login_attempt: Mutex<Option<LoginAttempt>>,
    auth_log_path: PathBuf,
    history_dir: PathBuf,
}

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

fn write_auth_log(path: &Path, stage: &str, message: &str) {
    let safe_message = message.replace(['\n', '\r'], " ");
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{} [{}] {}", now_ms(), stage, safe_message);
    }
    eprintln!("[splat3-auth:{stage}] {safe_message}");
}

fn auth_error(path: &Path, stage: &str, error: String) -> String {
    write_auth_log(path, stage, &error);
    format!("{error}\n认证日志：{}", path.display())
}

fn history_path(history_dir: &Path, account_id: &str) -> PathBuf {
    history_dir.join(format!("{account_id}.json"))
}

fn load_history_cache(
    history_dir: &Path,
    account_id: &str,
) -> Result<Option<HistoryCache>, String> {
    let path = history_path(history_dir, account_id);
    if !path.exists() {
        return Ok(None);
    }
    let contents =
        std::fs::read_to_string(&path).map_err(|error| format!("读取历史缓存失败：{error}"))?;
    let cache =
        serde_json::from_str(&contents).map_err(|error| format!("历史缓存格式无效：{error}"))?;
    Ok(Some(cache))
}

fn load_history_summary(
    history_dir: &Path,
    account_id: &str,
) -> Result<Option<HistorySyncSummary>, String> {
    Ok(load_history_cache(history_dir, account_id)?.map(|cache| cache.summary()))
}

fn save_history(history_dir: &Path, account_id: &str, cache: &HistoryCache) -> Result<(), String> {
    std::fs::create_dir_all(history_dir)
        .map_err(|error| format!("创建历史缓存目录失败：{error}"))?;
    // Compact JSON avoids storing formatting whitespace. The raw detail is
    // kept once per stable id and is merged on every overlapping sync.
    let contents =
        serde_json::to_string(cache).map_err(|error| format!("序列化历史缓存失败：{error}"))?;
    std::fs::write(
        history_path(history_dir, account_id),
        format!("{contents}\n"),
    )
    .map_err(|error| format!("保存历史缓存失败：{error}"))
}

#[tauri::command]
fn list_accounts(state: State<'_, AppState>) -> Result<Vec<AccountView>, String> {
    let registry = state
        .registry
        .lock()
        .map_err(|_| "account registry is locked")?;
    let active_id = registry.active_account_id();
    Ok(registry
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
        .collect::<Result<Vec<_>, String>>()?)
}

#[tauri::command]
fn begin_nintendo_login(state: State<'_, AppState>) -> Result<String, String> {
    let attempt = begin_login()?;
    let authorize_url = attempt.authorize_url.clone();
    *state
        .login_attempt
        .lock()
        .map_err(|_| "login state is locked")? = Some(attempt);
    Ok(authorize_url)
}

#[tauri::command]
async fn complete_nintendo_login(
    callback_url: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
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
    write_auth_log(
        &auth_log_path,
        "nintendo-session",
        "Nintendo session token 交换成功",
    );
    let timestamp = now_ms();

    let metadata = AccountMetadata {
        account_id: identity.account_id.clone(),
        display_name: identity.display_name,
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
    write_auth_log(
        &auth_log_path,
        "nintendo-session",
        "Nintendo Account 已保存，SplatNet token 延后获取",
    );
    registry.set_active(&identity.account_id, timestamp)
}

#[tauri::command]
async fn refresh_splatnet_tokens(
    account_id: String,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let auth_log_path = state.auth_log_path.clone();
    let (metadata, session_token) = {
        let registry = state
            .registry
            .lock()
            .map_err(|_| "account registry is locked")?;
        let metadata = registry
            .accounts()
            .iter()
            .find(|account| account.account_id == account_id)
            .cloned()
            .ok_or_else(|| format!("unknown account: {account_id}"))?;
        let secret = registry
            .get_secret(&account_id)?
            .ok_or_else(|| "该账号没有保存 Nintendo session token".to_owned())?;
        (metadata, secret.session_token)
    };

    write_auth_log(
        &auth_log_path,
        "splatnet-refresh",
        "使用已保存的 Nintendo session token 刷新 SplatNet token",
    );
    let identity = refresh_identity(&session_token)
        .await
        .map_err(|error| auth_error(&auth_log_path, "nintendo-session-refresh", error))?;
    let auth_config = SplatNetAuthConfig::default();
    write_auth_log(
        &auth_log_path,
        "f-config",
        &format!("使用 nxapi f 参数服务：{NXAPI_F_GENERATION_URL}"),
    );
    let splatnet_tokens = exchange_splatnet_tokens(&identity, &auth_config)
        .await
        .map_err(|error| auth_error(&auth_log_path, "splatnet-tokens", error))?;
    let secret = AccountSecret {
        session_token,
        game_web_token: Some(splatnet_tokens.game_web_token),
        bullet_token: Some(splatnet_tokens.bullet_token),
    };
    let mut metadata = metadata;
    metadata.display_name = identity.display_name;
    metadata.language = identity.language;
    metadata.country = identity.country;

    let mut registry = state
        .registry
        .lock()
        .map_err(|_| "account registry is locked")?;
    registry.upsert(metadata, secret)?;
    write_auth_log(
        &auth_log_path,
        "splatnet-tokens",
        "game web token 和 bullet token 获取成功",
    );
    Ok(())
}

#[tauri::command]
async fn sync_history(
    account_id: String,
    state: State<'_, AppState>,
) -> Result<HistorySyncSummary, String> {
    let auth_log_path = state.auth_log_path.clone();
    let history_dir = state.history_dir.clone();
    let (metadata, secret) = {
        let registry = state
            .registry
            .lock()
            .map_err(|_| "account registry is locked")?;
        let metadata = registry
            .accounts()
            .iter()
            .find(|account| account.account_id == account_id)
            .cloned()
            .ok_or_else(|| format!("unknown account: {account_id}"))?;
        let secret = registry
            .get_secret(&account_id)?
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

    write_auth_log(
        &auth_log_path,
        "history-sync",
        "开始增量同步：检查最近 50 条对战和 50 条打工记录",
    );
    let mut archive = load_history_cache(&history_dir, &account_id)
        .map_err(|error| auth_error(&auth_log_path, "history-load", error))?
        .unwrap_or(HistoryCache {
            schema_version: 1,
            fetched_at_ms: 0,
            battles: Vec::new(),
            coop: Vec::new(),
        });
    let known_battle_ids: HashSet<String> = archive
        .battles
        .iter()
        .map(|item| item.record.id.clone())
        .collect();
    let known_coop_ids: HashSet<String> = archive
        .coop
        .iter()
        .map(|item| item.record.id.clone())
        .collect();
    let client = SplatNetDataClient::connect(config)
        .await
        .map_err(|error| auth_error(&auth_log_path, "history-client", error))?;
    let cache = client
        .fetch_history_incremental(50, 50, &known_battle_ids, &known_coop_ids, now_ms())
        .await
        .map_err(|error| auth_error(&auth_log_path, "history-fetch", error))?;
    let (added_battle_count, added_coop_count) = archive.merge(cache);
    let mut summary = archive.summary();
    summary.added_battle_count = added_battle_count;
    summary.added_coop_count = added_coop_count;
    save_history(&history_dir, &account_id, &archive)
        .map_err(|error| auth_error(&auth_log_path, "history-save", error))?;
    write_auth_log(
        &auth_log_path,
        "history-sync",
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

#[tauri::command]
fn set_active_account(account_id: String, state: State<'_, AppState>) -> Result<(), String> {
    let mut registry = state
        .registry
        .lock()
        .map_err(|_| "account registry is locked")?;
    registry.set_active(&account_id, now_ms())
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let config_dir = app.path().app_config_dir()?;
            let metadata_path = config_dir.join("accounts.json");
            let registry = AccountRegistry::load(metadata_path, KeyringSecretStore::default())
                .map_err(std::io::Error::other)?;
            app.manage(AppState {
                registry: Mutex::new(registry),
                login_attempt: Mutex::new(None),
                auth_log_path: config_dir.join("auth.log"),
                history_dir: config_dir.join("history"),
            });

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_accounts,
            begin_nintendo_login,
            complete_nintendo_login,
            refresh_splatnet_tokens,
            sync_history,
            set_active_account,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Splat 3 Streaming Widget");
}
