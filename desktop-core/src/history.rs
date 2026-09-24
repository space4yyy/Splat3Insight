//! SplatNet 3 history client and the compact records used by the overlay.
//!
//! The client talks directly to SplatNet. It keeps one raw detail response per
//! stable SplatNet detail id together with a compact parsed record, so old data
//! can be re-parsed later without storing duplicate responses on every sync.

use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD};
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::Mutex as AsyncMutex;

use crate::splatnet_auth::{fetch_web_view_version, SPLATNET_URL};

const LATEST_BATTLE_QUERY: &str =
    "b24d22fd6cb251c515c2b90044039698aa27bc1fab15801d83014d919cd45780";
const X_BATTLE_HISTORIES_QUERY: &str =
    "eb5996a12705c2e94813a62e05c0dc419aad2811b8d49d53e5732290105559cb";
const BATTLE_DETAIL_QUERY: &str =
    "94faa2ff992222d11ced55e0f349920a82ac50f414ae33c83d1d1c9d8161c5dd";
const COOP_QUERY: &str = "e11a8cf2c3de7348495dea5cdcaa25e0c153541c4ed63f044b6c174bc5b703df";
const COOP_DETAIL_QUERY: &str = "f2d55873a9281213ae27edc171e2b19131b3021a2ae263757543cdd3bf015cc8";

const APP_USER_AGENT: &str = "Mozilla/5.0 (Linux; Android 14; Pixel 7a) AppleWebKit/537.36 Chrome/120.0.6099.230 Mobile Safari/537.36";
const MAX_NETWORK_RETRIES: usize = 3;
const MIN_REQUEST_INTERVAL: Duration = Duration::from_millis(200);
const MAX_RETRY_DELAY: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryFetchProgress {
    pub phase: String,
    pub completed: usize,
    pub total: usize,
    pub retry_attempt: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryErrorKind {
    TokenExpired,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryError {
    kind: HistoryErrorKind,
    message: String,
}

impl HistoryError {
    pub fn other(message: impl Into<String>) -> Self {
        Self {
            kind: HistoryErrorKind::Other,
            message: message.into(),
        }
    }

    pub fn token_expired(message: impl Into<String>) -> Self {
        Self {
            kind: HistoryErrorKind::TokenExpired,
            message: message.into(),
        }
    }

    pub fn kind(&self) -> HistoryErrorKind {
        self.kind
    }

    pub fn with_context(self, context: impl AsRef<str>) -> Self {
        Self {
            kind: self.kind,
            message: format!("{}{}", context.as_ref(), self.message),
        }
    }

    pub fn with_message(self, message: impl Into<String>) -> Self {
        Self {
            kind: self.kind,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for HistoryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for HistoryError {}

impl From<String> for HistoryError {
    fn from(message: String) -> Self {
        Self::other(message)
    }
}

pub type HistoryProgressCallback = Arc<dyn Fn(HistoryFetchProgress) + Send + Sync>;

#[derive(Debug, Clone)]
pub enum HistoryFetchedRecord {
    Battle(ArchivedBattle),
    Coop(ArchivedCoop),
    XBattleSettlement {
        record_id: String,
        power_after: f64,
        payload_hash: String,
        raw_group: Value,
    },
}

pub type HistoryRecordCallback =
    Arc<dyn Fn(HistoryFetchedRecord) -> Result<bool, String> + Send + Sync>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SplatNetDataConfig {
    pub game_web_token: String,
    pub bullet_token: String,
    pub language: String,
    pub country: String,
    pub web_view_version: Option<String>,
    pub app_user_agent: String,
}

impl SplatNetDataConfig {
    pub fn new(
        game_web_token: impl Into<String>,
        bullet_token: impl Into<String>,
        language: impl Into<String>,
        country: impl Into<String>,
    ) -> Self {
        Self {
            game_web_token: game_web_token.into(),
            bullet_token: bullet_token.into(),
            language: language.into(),
            country: country.into(),
            web_view_version: None,
            app_user_agent: APP_USER_AGENT.to_owned(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BattleRecord {
    pub id: String,
    pub played_time: Option<String>,
    pub mode: String,
    pub bankara_mode: Option<String>,
    pub rule: Option<String>,
    pub stage: Option<String>,
    pub judgement: Option<String>,
    pub player_name: Option<String>,
    pub weapon: Option<String>,
    pub paint: Option<i64>,
    pub power: Option<f64>,
    pub power_kind: Option<String>,
    pub kills: Option<i64>,
    pub assists: Option<i64>,
    pub deaths: Option<i64>,
    pub specials: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CoopRecord {
    pub id: String,
    pub played_time: Option<String>,
    pub rule: Option<String>,
    pub stage: Option<String>,
    pub result_wave: Option<i64>,
    pub clear_waves: Option<i64>,
    pub job_point: Option<f64>,
    pub after_grade: Option<String>,
    pub after_grade_point: Option<f64>,
    pub danger_rate: Option<f64>,
    pub golden_eggs: Option<i64>,
    pub power_eggs: Option<i64>,
    pub rescued: Option<i64>,
    pub rescued_by_others: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ArchivedBattle {
    pub record: BattleRecord,
    pub raw: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ArchivedCoop {
    pub record: CoopRecord,
    pub raw: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct HistoryCache {
    pub schema_version: u32,
    pub fetched_at_ms: i64,
    pub battles: Vec<ArchivedBattle>,
    pub coop: Vec<ArchivedCoop>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistorySyncSummary {
    pub battle_count: usize,
    pub coop_count: usize,
    pub fetched_at_ms: i64,
    pub added_battle_count: usize,
    pub added_coop_count: usize,
}

impl HistoryCache {
    pub fn summary(&self) -> HistorySyncSummary {
        HistorySyncSummary {
            battle_count: self.battles.len(),
            coop_count: self.coop.len(),
            fetched_at_ms: self.fetched_at_ms,
            added_battle_count: 0,
            added_coop_count: 0,
        }
    }

    /// Merge a freshly fetched window into the archive by SplatNet detail id.
    /// Existing records are replaced with the newest schema version, but are
    /// never duplicated or removed when the server window overlaps.
    pub fn merge(&mut self, incoming: HistoryCache) -> (usize, usize) {
        let fetched_at_ms = incoming.fetched_at_ms;
        let mut battle_index: HashMap<String, usize> = self
            .battles
            .iter()
            .enumerate()
            .map(|(index, item)| (item.record.id.clone(), index))
            .collect();
        let mut added_battles = 0;
        for incoming in incoming.battles {
            if let Some(index) = battle_index.get(&incoming.record.id).copied() {
                self.battles[index] = incoming;
            } else {
                battle_index.insert(incoming.record.id.clone(), self.battles.len());
                self.battles.push(incoming);
                added_battles += 1;
            }
        }

        let mut coop_index: HashMap<String, usize> = self
            .coop
            .iter()
            .enumerate()
            .map(|(index, item)| (item.record.id.clone(), index))
            .collect();
        let mut added_coop = 0;
        for incoming in incoming.coop {
            if let Some(index) = coop_index.get(&incoming.record.id).copied() {
                self.coop[index] = incoming;
            } else {
                coop_index.insert(incoming.record.id.clone(), self.coop.len());
                self.coop.push(incoming);
                added_coop += 1;
            }
        }

        self.schema_version = 1;
        self.fetched_at_ms = fetched_at_ms;
        self.battles
            .sort_by(|left, right| right.record.played_time.cmp(&left.record.played_time));
        self.coop
            .sort_by(|left, right| right.record.played_time.cmp(&left.record.played_time));
        (added_battles, added_coop)
    }
}

pub struct SplatNetDataClient {
    http: reqwest::Client,
    config: SplatNetDataConfig,
    web_view_version: String,
    request_gate: Arc<AsyncMutex<Instant>>,
}

impl SplatNetDataClient {
    pub async fn connect(mut config: SplatNetDataConfig) -> Result<Self, String> {
        if config.language.trim().is_empty() {
            config.language = "en-US".to_owned();
        }
        if config.country.trim().is_empty() {
            config.country = "US".to_owned();
        }
        let http = reqwest::Client::builder()
            .user_agent(&config.app_user_agent)
            .build()
            .map_err(|error| format!("创建 SplatNet 数据客户端失败：{error}"))?;
        let web_view_version = match config.web_view_version.clone() {
            Some(value) if !value.trim().is_empty() => value,
            _ => {
                fetch_web_view_version(
                    &http,
                    &config.game_web_token,
                    &config.language,
                    &config.country,
                    &config.app_user_agent,
                )
                .await
            }
        };

        Ok(Self {
            http,
            config,
            web_view_version,
            request_gate: Arc::new(AsyncMutex::new(Instant::now() - MIN_REQUEST_INTERVAL)),
        })
    }

    pub async fn fetch_history(
        &self,
        battle_limit: usize,
        coop_limit: usize,
        fetched_at_ms: i64,
    ) -> Result<HistoryCache, String> {
        self.fetch_history_with_limits(
            battle_limit,
            coop_limit,
            &HashSet::new(),
            &HashSet::new(),
            fetched_at_ms,
            None,
            None,
        )
        .await
    }

    /// Fetch every record exposed by the current SplatNet history response.
    ///
    /// The endpoint itself decides how much history is available to the
    /// client. We request the complete response on the first sync and keep
    /// the records locally, so later syncs only need to inspect the newest
    /// window.
    pub async fn fetch_history_full(&self, fetched_at_ms: i64) -> Result<HistoryCache, String> {
        self.fetch_history_with_limits(
            usize::MAX,
            usize::MAX,
            &HashSet::new(),
            &HashSet::new(),
            fetched_at_ms,
            None,
            None,
        )
        .await
    }

    pub async fn fetch_history_full_with_progress(
        &self,
        fetched_at_ms: i64,
        progress: HistoryProgressCallback,
    ) -> Result<HistoryCache, String> {
        self.fetch_history_full_with_progress_and_records(fetched_at_ms, progress, None)
            .await
    }

    pub async fn fetch_history_full_with_progress_and_records(
        &self,
        fetched_at_ms: i64,
        progress: HistoryProgressCallback,
        on_record: Option<HistoryRecordCallback>,
    ) -> Result<HistoryCache, String> {
        self.fetch_history_full_with_progress_and_records_from(
            &HashSet::new(),
            &HashSet::new(),
            fetched_at_ms,
            progress,
            on_record,
        )
        .await
    }

    /// Continue a full sync from records that were already committed by a
    /// previous attempt. The list endpoint is still read in full, but detail
    /// requests for committed IDs are skipped.
    pub async fn fetch_history_full_with_progress_and_records_from(
        &self,
        known_battle_ids: &HashSet<String>,
        known_coop_ids: &HashSet<String>,
        fetched_at_ms: i64,
        progress: HistoryProgressCallback,
        on_record: Option<HistoryRecordCallback>,
    ) -> Result<HistoryCache, String> {
        self.fetch_history_with_limits(
            usize::MAX,
            usize::MAX,
            known_battle_ids,
            known_coop_ids,
            fetched_at_ms,
            Some(&progress),
            on_record.as_ref(),
        )
        .await
    }

    pub async fn fetch_history_full_with_progress_and_records_from_typed(
        &self,
        known_battle_ids: &HashSet<String>,
        known_coop_ids: &HashSet<String>,
        fetched_at_ms: i64,
        progress: HistoryProgressCallback,
        on_record: Option<HistoryRecordCallback>,
    ) -> Result<HistoryCache, HistoryError> {
        self.fetch_history_with_limits_typed(
            usize::MAX,
            usize::MAX,
            known_battle_ids,
            known_coop_ids,
            fetched_at_ms,
            Some(&progress),
            on_record.as_ref(),
        )
        .await
    }

    /// Fetch the latest server windows but request detail records only for IDs
    /// that are not already present in the local archive.
    pub async fn fetch_history_incremental(
        &self,
        battle_limit: usize,
        coop_limit: usize,
        known_battle_ids: &HashSet<String>,
        known_coop_ids: &HashSet<String>,
        fetched_at_ms: i64,
    ) -> Result<HistoryCache, String> {
        self.fetch_history_with_limits(
            battle_limit,
            coop_limit,
            known_battle_ids,
            known_coop_ids,
            fetched_at_ms,
            None,
            None,
        )
        .await
    }

    pub async fn fetch_history_incremental_with_progress(
        &self,
        battle_limit: usize,
        coop_limit: usize,
        known_battle_ids: &HashSet<String>,
        known_coop_ids: &HashSet<String>,
        fetched_at_ms: i64,
        progress: HistoryProgressCallback,
    ) -> Result<HistoryCache, String> {
        self.fetch_history_incremental_with_progress_and_records(
            battle_limit,
            coop_limit,
            known_battle_ids,
            known_coop_ids,
            fetched_at_ms,
            progress,
            None,
        )
        .await
    }

    pub async fn fetch_history_incremental_with_progress_and_records(
        &self,
        battle_limit: usize,
        coop_limit: usize,
        known_battle_ids: &HashSet<String>,
        known_coop_ids: &HashSet<String>,
        fetched_at_ms: i64,
        progress: HistoryProgressCallback,
        on_record: Option<HistoryRecordCallback>,
    ) -> Result<HistoryCache, String> {
        self.fetch_history_with_limits(
            battle_limit,
            coop_limit,
            known_battle_ids,
            known_coop_ids,
            fetched_at_ms,
            Some(&progress),
            on_record.as_ref(),
        )
        .await
    }

    pub async fn fetch_history_incremental_with_progress_and_records_typed(
        &self,
        battle_limit: usize,
        coop_limit: usize,
        known_battle_ids: &HashSet<String>,
        known_coop_ids: &HashSet<String>,
        fetched_at_ms: i64,
        progress: HistoryProgressCallback,
        on_record: Option<HistoryRecordCallback>,
    ) -> Result<HistoryCache, HistoryError> {
        self.fetch_history_with_limits_typed(
            battle_limit,
            coop_limit,
            known_battle_ids,
            known_coop_ids,
            fetched_at_ms,
            Some(&progress),
            on_record.as_ref(),
        )
        .await
    }

    async fn fetch_history_with_limits(
        &self,
        battle_limit: usize,
        coop_limit: usize,
        known_battle_ids: &HashSet<String>,
        known_coop_ids: &HashSet<String>,
        fetched_at_ms: i64,
        progress: Option<&HistoryProgressCallback>,
        on_record: Option<&HistoryRecordCallback>,
    ) -> Result<HistoryCache, String> {
        self.fetch_history_with_limits_typed(
            battle_limit,
            coop_limit,
            known_battle_ids,
            known_coop_ids,
            fetched_at_ms,
            progress,
            on_record,
        )
        .await
        .map_err(|error| error.to_string())
    }

    async fn fetch_history_with_limits_typed(
        &self,
        battle_limit: usize,
        coop_limit: usize,
        known_battle_ids: &HashSet<String>,
        known_coop_ids: &HashSet<String>,
        fetched_at_ms: i64,
        progress: Option<&HistoryProgressCallback>,
        on_record: Option<&HistoryRecordCallback>,
    ) -> Result<HistoryCache, HistoryError> {
        notify_progress(progress, "battle-list", 0, 0, 0);
        let listed_battle_ids = self
            .fetch_ids(
                LATEST_BATTLE_QUERY,
                "latestBattleHistories",
                battle_limit,
                None,
                progress,
                "battle-list",
            )
            .await
            .map_err(|error| error.with_context("对战列表查询失败："))?;
        let battle_ids = new_ids(listed_battle_ids, known_battle_ids);

        notify_progress(progress, "coop-list", 0, 0, 0);
        let coop_ids = self
            .fetch_ids(
                COOP_QUERY,
                "coopResult",
                coop_limit,
                Some("en-US"),
                progress,
                "coop-list",
            )
            .await
            .map_err(|error| error.with_context("打工列表查询失败："))?;
        let coop_ids = new_ids(coop_ids, known_coop_ids);

        let mut battles = Vec::with_capacity(battle_ids.len());
        let battle_total = battle_ids.len();
        notify_progress(progress, "battle-details", 0, battle_total, 0);
        for (index, id) in battle_ids.into_iter().enumerate() {
            if let Some(detail) = self
                .fetch_detail(
                    BATTLE_DETAIL_QUERY,
                    "vsResultId",
                    &id,
                    None,
                    progress,
                    "battle-details",
                )
                .await
                .map_err(|error| {
                    error.with_context(format!("对战详情查询失败（第 {} 条）：", battles.len() + 1))
                })?
            {
                if let Some(record) = parse_battle(&detail, &id) {
                    let archived = ArchivedBattle {
                        record,
                        raw: detail,
                    };
                    if let Some(on_record) = on_record {
                        on_record(HistoryFetchedRecord::Battle(archived.clone())).map_err(
                            |error| format!("保存对战详情失败（第 {} 条）：{error}", index + 1),
                        )?;
                    }
                    battles.push(archived);
                }
            }
            notify_progress(progress, "battle-details", index + 1, battle_total, 0);
        }

        notify_progress(progress, "x-measurements", 0, 1, 0);
        let x_battle_response = self
            .graphql(
                X_BATTLE_HISTORIES_QUERY,
                json!({}),
                None,
                progress,
                "x-measurements",
            )
            .await
            .map_err(|error| error.with_context("X 赛结算查询失败："))?;
        let mut available_battle_ids = known_battle_ids.clone();
        available_battle_ids.extend(battles.iter().map(|battle| battle.record.id.clone()));
        let x_settlements =
            completed_x_power_settlements(&x_battle_response, &available_battle_ids)
                .map_err(HistoryError::other)?;
        notify_progress(progress, "x-measurements", 1, 1, 0);

        if let Some(on_record) = on_record {
            for settlement in x_settlements {
                on_record(HistoryFetchedRecord::XBattleSettlement {
                    record_id: settlement.record_id,
                    power_after: settlement.power_after,
                    payload_hash: settlement.payload_hash,
                    raw_group: settlement.raw_group,
                })
                .map_err(|error| format!("保存 X 赛结算数据失败：{error}"))?;
            }
        }

        let mut coop = Vec::with_capacity(coop_ids.len());
        let coop_total = coop_ids.len();
        notify_progress(progress, "coop-details", 0, coop_total, 0);
        for (index, id) in coop_ids.into_iter().enumerate() {
            if let Some(detail) = self
                .fetch_detail(
                    COOP_DETAIL_QUERY,
                    "coopHistoryDetailId",
                    &id,
                    Some("en-US"),
                    progress,
                    "coop-details",
                )
                .await
                .map_err(|error| {
                    error.with_context(format!("打工详情查询失败（第 {} 条）：", coop.len() + 1))
                })?
            {
                if let Some(record) = parse_coop(&detail, &id) {
                    let archived = ArchivedCoop {
                        record,
                        raw: detail,
                    };
                    if let Some(on_record) = on_record {
                        on_record(HistoryFetchedRecord::Coop(archived.clone())).map_err(
                            |error| format!("保存打工详情失败（第 {} 条）：{error}", index + 1),
                        )?;
                    }
                    coop.push(archived);
                }
            }
            notify_progress(progress, "coop-details", index + 1, coop_total, 0);
        }

        battles.sort_by(|left, right| right.record.played_time.cmp(&left.record.played_time));
        coop.sort_by(|left, right| right.record.played_time.cmp(&left.record.played_time));

        Ok(HistoryCache {
            schema_version: 1,
            fetched_at_ms,
            battles,
            coop,
        })
    }

    async fn fetch_ids(
        &self,
        query_hash: &str,
        root_key: &str,
        limit: usize,
        language: Option<&str>,
        progress: Option<&HistoryProgressCallback>,
        progress_phase: &str,
    ) -> Result<Vec<String>, HistoryError> {
        let value = self
            .graphql(query_hash, json!({}), language, progress, progress_phase)
            .await?;
        let root = value
            .get("data")
            .and_then(|data| data.get(root_key))
            .ok_or_else(|| format!("SplatNet 返回中缺少 {root_key}"))?;
        let mut ids = Vec::new();
        let mut seen = HashSet::new();
        let groups = root
            .get("historyGroups")
            .and_then(|value| value.get("nodes"))
            .and_then(Value::as_array)
            .ok_or_else(|| format!("SplatNet {root_key} 返回中缺少 historyGroups"))?;

        for group in groups {
            let details = group
                .get("historyDetails")
                .and_then(|value| value.get("nodes"))
                .and_then(Value::as_array)
                .ok_or_else(|| format!("SplatNet {root_key} 返回中缺少 historyDetails"))?;
            for detail in details {
                let Some(id) = detail.get("id").and_then(Value::as_str) else {
                    continue;
                };
                if seen.insert(id.to_owned()) {
                    ids.push(id.to_owned());
                    if ids.len() >= limit {
                        return Ok(ids);
                    }
                }
            }
        }
        Ok(ids)
    }

    async fn fetch_detail(
        &self,
        query_hash: &str,
        variable_name: &str,
        id: &str,
        language: Option<&str>,
        progress: Option<&HistoryProgressCallback>,
        progress_phase: &str,
    ) -> Result<Option<Value>, HistoryError> {
        let value = self
            .graphql(
                query_hash,
                json!({ variable_name: id }),
                language,
                progress,
                progress_phase,
            )
            .await?;
        let data = value.get("data").cloned().unwrap_or(Value::Null);
        let detail = if variable_name == "vsResultId" {
            data.get("vsHistoryDetail")
        } else {
            data.get("coopHistoryDetail")
        };
        Ok(match detail {
            Some(Value::Null) | None => None,
            Some(value) => Some(value.clone()),
        })
    }

    async fn wait_for_request_slot(&self) {
        let mut next_allowed = self.request_gate.lock().await;
        let now = Instant::now();
        if *next_allowed > now {
            tokio::time::sleep(*next_allowed - now).await;
        }
        *next_allowed = Instant::now() + MIN_REQUEST_INTERVAL;
    }

    async fn graphql(
        &self,
        query_hash: &str,
        variables: Value,
        language: Option<&str>,
        progress: Option<&HistoryProgressCallback>,
        progress_phase: &str,
    ) -> Result<Value, HistoryError> {
        let language = language.unwrap_or(&self.config.language);
        let request_body = json!({
            "extensions": { "persistedQuery": { "sha256Hash": query_hash, "version": 1 } },
            "variables": variables,
        });
        let mut retry_attempt = 0;
        let (status, content_type, request_id, response_body) = loop {
            self.wait_for_request_slot().await;
            let response = self
                .http
                .post(format!("{SPLATNET_URL}/api/graphql"))
                .header(
                    "Authorization",
                    format!("Bearer {}", self.config.bullet_token),
                )
                .header("Accept-Language", language)
                .header("User-Agent", &self.config.app_user_agent)
                .header("X-Web-View-Ver", &self.web_view_version)
                .header("Content-Type", "application/json")
                .header("Accept", "*/*")
                .header("Origin", SPLATNET_URL)
                .header("X-Requested-With", "com.nintendo.znca")
                .header(
                    "Referer",
                    format!(
                        "{SPLATNET_URL}?lang={language}&na_country={}&na_lang={language}",
                        self.config.country
                    ),
                )
                .header(
                    "Cookie",
                    format!("_gtoken={}; _dnt=1", self.config.game_web_token),
                )
                .json(&request_body)
                .send()
                .await;

            let response = match response {
                Ok(response) => response,
                Err(error) if is_retryable_network_error(&error) => {
                    if retry_attempt >= MAX_NETWORK_RETRIES {
                        return Err(HistoryError::other(format!(
                            "SplatNet 网络请求失败，已重试 {} 次：{}",
                            retry_attempt, error
                        )));
                    }
                    retry_attempt += 1;
                    notify_retry(progress, progress_phase, retry_attempt);
                    tokio::time::sleep(retry_delay(retry_attempt, None)).await;
                    continue;
                }
                Err(error) => {
                    return Err(HistoryError::other(format!(
                        "SplatNet 网络请求失败：{error}"
                    )))
                }
            };
            let status = response.status();
            if status == reqwest::StatusCode::UNAUTHORIZED {
                return Err(HistoryError::token_expired(
                    "SplatNet bullet token 已失效，请重新获取 SplatNet 权限",
                ));
            }
            let retry_after = parse_retry_after(response.headers());
            let content_type = response
                .headers()
                .get(reqwest::header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .unwrap_or("unknown")
                .to_owned();
            let request_id = response
                .headers()
                .get("x-request-id")
                .or_else(|| response.headers().get("x-trace-id"))
                .and_then(|value| value.to_str().ok())
                .unwrap_or("none")
                .to_owned();
            let response_body = match response.bytes().await {
                Ok(body) => body,
                Err(_error) if retry_attempt < MAX_NETWORK_RETRIES => {
                    retry_attempt += 1;
                    notify_retry(progress, progress_phase, retry_attempt);
                    tokio::time::sleep(retry_delay(retry_attempt, retry_after)).await;
                    continue;
                }
                Err(error) => {
                    return Err(HistoryError::other(format!(
                        "读取 SplatNet 数据响应失败（HTTP {status}，已重试 {} 次）：{error}",
                        retry_attempt
                    )));
                }
            };

            if is_retryable_status(status) && retry_attempt < MAX_NETWORK_RETRIES {
                retry_attempt += 1;
                notify_retry(progress, progress_phase, retry_attempt);
                tokio::time::sleep(retry_delay(retry_attempt, retry_after)).await;
                continue;
            }
            break (status, content_type, request_id, response_body);
        };
        parse_graphql_response(
            status,
            &content_type,
            &request_id,
            &response_body,
            retry_attempt,
        )
    }
}

fn parse_graphql_response(
    status: reqwest::StatusCode,
    content_type: &str,
    request_id: &str,
    response_body: &[u8],
    retry_attempt: usize,
) -> Result<Value, HistoryError> {
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Err(HistoryError::token_expired(
            "SplatNet bullet token 已失效，请重新获取 SplatNet 权限",
        ));
    }
    if status == reqwest::StatusCode::FORBIDDEN {
        return Err(HistoryError::other(
            "SplatNet 拒绝了数据请求，客户端版本或 Web View 版本可能已过期",
        ));
    }
    if !status.is_success() {
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            return Err(HistoryError::other(format!(
                "SplatNet 请求过于频繁（HTTP 429），已重试 {retry_attempt} 次，请稍后再试"
            )));
        }
        if status.is_server_error() {
            return Err(HistoryError::other(format!(
                "SplatNet 服务暂时不可用（HTTP {status}），已重试 {retry_attempt} 次，请稍后再试；Request-ID: {request_id}"
            )));
        }
        return Err(HistoryError::other(format!(
            "SplatNet 数据请求失败（HTTP {status}，Content-Type: {content_type}，Request-ID: {request_id}，响应：{}）",
            response_preview(response_body)
        )));
    }

    let value: Value = serde_json::from_slice(response_body).map_err(|_| {
        HistoryError::other(format!(
            "SplatNet 数据响应不是有效 JSON（HTTP {status}，Content-Type: {content_type}，Request-ID: {request_id}，响应：{}）",
            response_preview(response_body)
        ))
    })?;
    if let Some(first) = value
        .get("errors")
        .and_then(Value::as_array)
        .and_then(|errors| errors.first())
    {
        let error_message = format!("SplatNet GraphQL 返回错误：{}", compact_json(first));
        if graphql_error_is_unauthorized(first) {
            return Err(HistoryError::token_expired(error_message));
        }
        return Err(HistoryError::other(error_message));
    }
    if value.get("data").is_none() {
        return Err(HistoryError::other("SplatNet 响应缺少 data"));
    }
    Ok(value)
}

fn notify_progress(
    progress: Option<&HistoryProgressCallback>,
    phase: &str,
    completed: usize,
    total: usize,
    retry_attempt: usize,
) {
    if let Some(progress) = progress {
        progress(HistoryFetchProgress {
            phase: phase.to_owned(),
            completed,
            total,
            retry_attempt,
        });
    }
}

fn notify_retry(
    progress: Option<&HistoryProgressCallback>,
    progress_phase: &str,
    retry_attempt: usize,
) {
    let phase = format!("retrying:{progress_phase}");
    notify_progress(progress, &phase, 0, 0, retry_attempt);
}

fn is_retryable_network_error(error: &reqwest::Error) -> bool {
    error.is_timeout() || error.is_connect()
}

fn is_retryable_status(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::TOO_MANY_REQUESTS || status.is_server_error()
}

fn graphql_error_is_unauthorized(error: &Value) -> bool {
    matches!(
        error
            .pointer("/extensions/code")
            .and_then(Value::as_str)
            .map(str::to_ascii_uppercase)
            .as_deref(),
        Some("UNAUTHENTICATED") | Some("UNAUTHORIZED") | Some("AUTHENTICATION_ERROR")
    )
}

fn parse_retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    let seconds = headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.trim().parse::<u64>().ok())?;
    (seconds > 0).then(|| Duration::from_secs(seconds).min(MAX_RETRY_DELAY))
}

fn retry_delay(attempt: usize, retry_after: Option<Duration>) -> Duration {
    if let Some(retry_after) = retry_after {
        return retry_after.min(MAX_RETRY_DELAY);
    }
    let exponent = attempt.saturating_sub(1).min(5) as u32;
    let delay = Duration::from_millis(500_u64.saturating_mul(2_u64.pow(exponent)));
    delay.min(MAX_RETRY_DELAY)
}

fn new_ids(ids: Vec<String>, known_ids: &HashSet<String>) -> Vec<String> {
    ids.into_iter()
        .filter(|id| !known_ids.contains(id))
        .collect()
}

#[derive(Debug, Clone, PartialEq)]
struct XPowerSettlement {
    record_id: String,
    power_after: f64,
    payload_hash: String,
    raw_group: Value,
}

fn completed_x_power_settlements(
    response: &Value,
    record_ids: &HashSet<String>,
) -> Result<Vec<XPowerSettlement>, String> {
    let groups = response
        .pointer("/data/xBattleHistories/historyGroups/nodes")
        .and_then(Value::as_array)
        .ok_or_else(|| "SplatNet 返回中缺少 X 赛测定组".to_owned())?;
    let mut available_ids = HashMap::new();
    for id in record_ids {
        available_ids
            .entry(normalize_x_battle_id(id))
            .or_insert_with(|| id.clone());
    }

    let mut settlements = Vec::new();
    let mut seen_ids = HashSet::new();
    for group in groups {
        let Some(measurement) = group.get("xMatchMeasurement") else {
            continue;
        };
        if measurement.get("state").and_then(Value::as_str) != Some("COMPLETED") {
            continue;
        }
        let Some(power_after) = measurement.get("xPowerAfter").and_then(Value::as_f64) else {
            continue;
        };
        let Some(leader_id) = group
            .pointer("/historyDetails/nodes/0/id")
            .and_then(Value::as_str)
        else {
            continue;
        };
        let Some(record_id) = available_ids.get(&normalize_x_battle_id(leader_id)) else {
            continue;
        };
        if !seen_ids.insert(record_id.clone()) {
            continue;
        }
        let serialized_group =
            serde_json::to_vec(group).map_err(|error| format!("序列化 X 赛测定组失败：{error}"))?;
        let payload_hash = Sha256::digest(serialized_group)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        settlements.push(XPowerSettlement {
            record_id: record_id.clone(),
            power_after,
            payload_hash,
            raw_group: group.clone(),
        });
    }
    Ok(settlements)
}

fn normalize_x_battle_id(id: &str) -> String {
    let decoded = [
        STANDARD.decode(id),
        STANDARD_NO_PAD.decode(id),
        URL_SAFE.decode(id),
        URL_SAFE_NO_PAD.decode(id),
    ]
    .into_iter()
    .find_map(Result::ok)
    .and_then(|bytes| String::from_utf8(bytes).ok())
    .unwrap_or_else(|| id.to_owned());
    decoded.replace("XMATCH", "RECENT")
}

fn parse_battle(detail: &Value, fallback_id: &str) -> Option<BattleRecord> {
    let id = string_at(detail, &["id"]).unwrap_or_else(|| fallback_id.to_owned());
    let raw_mode = string_at(detail, &["vsMode", "mode"]).unwrap_or_else(|| "BATTLE".to_owned());
    let bankara_mode = string_at(detail, &["bankaraMatch", "mode"]);
    let mode = if bankara_mode.is_none() && is_private_battle_mode(&raw_mode) {
        "PRIVATE_BATTLE".to_owned()
    } else if bankara_mode.is_none()
        && (is_event_battle_mode(&raw_mode)
            || ["eventMatch", "eventBattle", "event", "leagueMatch"]
                .iter()
                .any(|key| detail.get(*key).is_some_and(Value::is_object)))
    {
        "EVENT_MATCH".to_owned()
    } else {
        raw_mode
    };
    let player = detail
        .get("myTeam")
        .and_then(|value| value.get("players"))
        .and_then(Value::as_array)
        .and_then(|players| {
            players
                .iter()
                .find(|player| player.get("isMyself").and_then(Value::as_bool) == Some(true))
                .or_else(|| players.first())
        });
    let result = player.and_then(|value| value.get("result"));

    let (power, power_kind) = match mode.as_str() {
        "X_MATCH" => (number_at(detail, &["xMatch", "lastXPower"]), Some("x")),
        "BANKARA" if bankara_mode.as_deref() == Some("CHALLENGE") => (
            number_at(detail, &["bankaraMatch", "weaponPower"]),
            Some("weapon"),
        ),
        "BANKARA" => (
            number_at(detail, &["bankaraMatch", "bankaraPower", "power"]),
            Some("team"),
        ),
        "EVENT_MATCH" => (
            [
                &["eventMatch", "eventPower"][..],
                &["eventMatch", "power"],
                &["eventBattle", "eventPower"],
                &["eventBattle", "power"],
                &["challenge", "eventPower"],
                &["challenge", "power"],
                &["challengePower"],
                &["event", "eventPower"],
                &["event", "power"],
                &["leagueMatch", "eventPower"],
                &["leagueMatch", "power"],
                &["eventPower"],
            ]
            .iter()
            .find_map(|path| number_at(detail, path)),
            Some("event"),
        ),
        _ => (None, None),
    };

    Some(BattleRecord {
        id,
        played_time: string_at(detail, &["playedTime"]),
        mode,
        bankara_mode,
        rule: string_at(detail, &["vsRule", "name"])
            .or_else(|| string_at(detail, &["vsRule", "rule"])),
        stage: string_at(detail, &["vsStage", "name"]),
        judgement: string_at(detail, &["judgement"]),
        player_name: player.and_then(|value| string_at(value, &["name"])),
        weapon: player.and_then(|value| string_at(value, &["weapon", "name"])),
        paint: player.and_then(|value| integer_at(value, &["paint"])),
        power,
        power_kind: power_kind.map(str::to_owned),
        kills: result.and_then(|value| integer_at(value, &["kill"])),
        assists: result.and_then(|value| integer_at(value, &["assist"])),
        deaths: result.and_then(|value| integer_at(value, &["death"])),
        specials: result.and_then(|value| integer_at(value, &["special"])),
    })
}

fn is_event_battle_mode(mode: &str) -> bool {
    matches!(mode, "EVENT" | "EVENT_MATCH" | "CHALLENGE" | "LEAGUE")
}

fn is_private_battle_mode(mode: &str) -> bool {
    matches!(mode, "PRIVATE" | "PRIVATE_MATCH" | "PRIVATE_BATTLE")
}

fn parse_coop(detail: &Value, fallback_id: &str) -> Option<CoopRecord> {
    let id = string_at(detail, &["id"]).unwrap_or_else(|| fallback_id.to_owned());
    let result_wave = integer_at(detail, &["resultWave"]);
    let clear_waves = result_wave.map(|value| {
        let value = value - 1;
        if value == -1 {
            3
        } else {
            value
        }
    });
    let player = detail
        .get("myResult")
        .or_else(|| detail.get("myPlayer"))
        .or_else(|| detail.get("playerResult"));
    let (golden_eggs, power_eggs, rescued, rescued_by_others) = player
        .map(|value| {
            (
                integer_at(value, &["goldenEgg"]).or_else(|| integer_at(value, &["goldenEggs"])),
                integer_at(value, &["powerEgg"]).or_else(|| integer_at(value, &["powerEggs"])),
                integer_at(value, &["rescued"]),
                integer_at(value, &["rescuedByOthers"]),
            )
        })
        .unwrap_or((None, None, None, None));

    Some(CoopRecord {
        id,
        played_time: string_at(detail, &["playedTime"]),
        rule: string_at(detail, &["rule"]),
        stage: string_at(detail, &["coopStage", "name"]),
        result_wave,
        clear_waves,
        job_point: number_at(detail, &["jobPoint"]),
        after_grade: string_at(detail, &["afterGrade", "name"]),
        after_grade_point: number_at(detail, &["afterGradePoint"]),
        danger_rate: number_at(detail, &["dangerRate"]),
        golden_eggs,
        power_eggs,
        rescued,
        rescued_by_others,
    })
}

fn string_at(value: &Value, path: &[&str]) -> Option<String> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    current.as_str().map(ToOwned::to_owned)
}

fn number_at(value: &Value, path: &[&str]) -> Option<f64> {
    let mut current = value;
    for key in path {
        current = current.get(*key)?;
    }
    if let Some(number) = current.as_f64() {
        return Some(number);
    }
    current.get("power").and_then(Value::as_f64)
}

fn integer_at(value: &Value, path: &[&str]) -> Option<i64> {
    let number = number_at(value, path)?;
    if number.fract() == 0.0 {
        Some(number as i64)
    } else {
        None
    }
}

fn compact_json(value: &Value) -> String {
    let mut sanitized = value.clone();
    redact_sensitive_fields(&mut sanitized);
    serde_json::to_string(&sanitized).unwrap_or_else(|_| "未知 GraphQL 错误".to_owned())
}

fn response_preview(body: &[u8]) -> String {
    if body.is_empty() {
        return "<empty body, 0 bytes>".to_owned();
    }

    let Ok(mut value) = serde_json::from_slice::<Value>(body) else {
        return format!("<non-JSON response omitted, {} bytes>", body.len());
    };
    redact_sensitive_fields(&mut value);
    let text =
        serde_json::to_string(&value).unwrap_or_else(|_| "<JSON response omitted>".to_owned());
    let mut chars = text.chars();
    let preview: String = chars.by_ref().take(240).collect();
    if chars.next().is_some() {
        format!("{preview}…")
    } else {
        preview
    }
}

fn redact_sensitive_fields(value: &mut Value) {
    match value {
        Value::Object(object) => {
            for (key, value) in object.iter_mut() {
                let normalized: String = key
                    .chars()
                    .filter(|character| character.is_ascii_alphanumeric())
                    .map(|character| character.to_ascii_lowercase())
                    .collect();
                let sensitive = normalized.contains("token")
                    || normalized.contains("authorization")
                    || normalized.contains("cookie")
                    || normalized.contains("password")
                    || normalized.contains("secret")
                    || normalized.contains("verifier")
                    || normalized == "f";
                if sensitive {
                    *value = Value::String("[REDACTED]".to_owned());
                } else {
                    redact_sensitive_fields(value);
                }
            }
        }
        Value::Array(values) => {
            for value in values {
                redact_sensitive_fields(value);
            }
        }
        Value::String(text) => {
            let normalized = text.to_ascii_lowercase();
            let credential_marker = [
                "token=",
                "token:",
                "session_token",
                "authorization",
                "bearer ",
                "cookie:",
                "password=",
                "secret=",
                "verifier=",
                "f=",
            ]
            .iter()
            .any(|marker| normalized.contains(marker));
            let looks_like_opaque_credential = text.len() > 32
                && !text.chars().any(char::is_whitespace)
                && text.chars().all(|character| {
                    character.is_ascii_alphanumeric() || "-_.+/=".contains(character)
                });
            if credential_marker || looks_like_opaque_credential {
                *text = "[REDACTED]".to_owned();
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::STANDARD;
    use base64::Engine;

    #[test]
    fn token_expiration_remains_typed_through_error_context() {
        let error = HistoryError::token_expired("HTTP 401")
            .with_context("对战列表查询失败：")
            .with_message("账号 peanuts：对战列表查询失败：HTTP 401");
        assert_eq!(error.kind(), HistoryErrorKind::TokenExpired);
        assert!(error.to_string().contains("HTTP 401"));
        assert_eq!(
            HistoryError::other("HTTP 429").kind(),
            HistoryErrorKind::Other
        );
    }

    #[test]
    fn only_explicit_graphql_auth_codes_mark_a_token_expired() {
        assert!(graphql_error_is_unauthorized(&json!({
            "extensions": { "code": "UNAUTHENTICATED" }
        })));
        assert!(!graphql_error_is_unauthorized(&json!({
            "extensions": { "code": "RATE_LIMITED" }
        })));
    }

    #[test]
    fn parses_x_battle_with_assists_and_power() {
        let detail = json!({
            "id": "battle-1",
            "vsMode": { "mode": "X_MATCH" },
            "vsRule": { "name": "CLAM" },
            "vsStage": { "name": "MakoMart" },
            "judgement": "WIN",
            "playedTime": "2026-09-20T00:09:47Z",
            "myTeam": { "players": [{
                "isMyself": true, "name": "Player", "paint": 1369,
                "weapon": { "name": "Splat Dualies" },
                "result": { "kill": 8, "assist": 4, "death": 8, "special": 5 }
            }]},
            "xMatch": { "lastXPower": 2100.1039 }
        });

        let record = parse_battle(&detail, "fallback").unwrap();
        assert_eq!(record.mode, "X_MATCH");
        assert_eq!(record.power, Some(2100.1039));
        assert_eq!(record.assists, Some(4));
        assert_eq!(record.power_kind.as_deref(), Some("x"));
    }

    #[test]
    fn completed_x_measurement_replaces_immutable_battle_power_for_matching_record() {
        let recent_id = STANDARD.encode("VsHistoryDetail-RECENT-123");
        let x_match_id = STANDARD.encode("VsHistoryDetail-XMATCH-123");
        let known_ids = HashSet::from([recent_id.clone()]);
        let response = json!({
            "data": { "xBattleHistories": { "historyGroups": { "nodes": [
                {
                    "historyDetails": { "nodes": [
                        { "id": x_match_id },
                        { "id": STANDARD.encode("VsHistoryDetail-XMATCH-122") }
                    ]},
                    "xMatchMeasurement": {
                        "state": "COMPLETED",
                        "winCount": 3,
                        "loseCount": 0,
                        "xPowerAfter": 2057.026
                    }
                },
                {
                    "historyDetails": { "nodes": [{ "id": STANDARD.encode("VsHistoryDetail-XMATCH-100") }] },
                    "xMatchMeasurement": {
                        "state": "IN_PROGRESS",
                        "xPowerAfter": 9999.0
                    }
                }
            ]}}}
        });

        let settlements = completed_x_power_settlements(&response, &known_ids).unwrap();

        assert_eq!(settlements.len(), 1);
        assert_eq!(settlements[0].record_id, recent_id);
        assert_eq!(settlements[0].power_after, 2057.026);
        assert_eq!(settlements[0].payload_hash.len(), 64);
        assert_eq!(
            settlements[0].raw_group["xMatchMeasurement"]["state"],
            "COMPLETED"
        );
    }

    #[test]
    fn parses_bankara_open_and_challenge_power_shapes() {
        let open = json!({
            "vsMode": { "mode": "BANKARA" },
            "bankaraMatch": { "mode": "OPEN", "bankaraPower": { "power": 1878.3 } }
        });
        let challenge = json!({
            "vsMode": { "mode": "BANKARA" },
            "bankaraMatch": { "mode": "CHALLENGE", "weaponPower": { "power": 2019.8 } }
        });
        assert_eq!(
            parse_battle(&open, "open").unwrap().power_kind.as_deref(),
            Some("team")
        );
        assert_eq!(
            parse_battle(&challenge, "challenge").unwrap().power,
            Some(2019.8)
        );
    }

    #[test]
    fn parses_event_match_and_event_power() {
        let detail = json!({
            "id": "event-battle-1",
            "vsMode": { "mode": "CHALLENGE" },
            "vsRule": { "name": "LOFT" },
            "eventMatch": {
                "eventId": "event-2026-09",
                "timeSlotId": "slot-2",
                "isSolo": true,
                "eventPower": { "power": 1842.6 }
            },
            "myTeam": { "players": [{
                "isMyself": true,
                "result": { "kill": 7, "assist": 2, "death": 4 }
            }]}
        });

        let record = parse_battle(&detail, "fallback").unwrap();
        assert_eq!(record.mode, "EVENT_MATCH");
        assert_eq!(record.power, Some(1842.6));
        assert_eq!(record.power_kind.as_deref(), Some("event"));
        assert_eq!(record.rule.as_deref(), Some("LOFT"));
    }

    #[test]
    fn parses_private_battle_without_power() {
        let detail = json!({
            "id": "private-battle-1",
            "vsMode": { "mode": "PRIVATE" },
            "vsRule": { "name": "LOFT" },
            "privateBattle": { "power": 2400.0 },
            "myTeam": { "players": [{
                "isMyself": true,
                "result": { "kill": 8, "assist": 3, "death": 4 }
            }]}
        });

        let record = parse_battle(&detail, "fallback").unwrap();
        assert_eq!(record.mode, "PRIVATE_BATTLE");
        assert_eq!(record.power, None);
        assert_eq!(record.power_kind, None);
        assert_eq!(record.kills, Some(8));
        assert_eq!(record.assists, Some(3));
        assert_eq!(record.deaths, Some(4));
    }

    #[test]
    fn parses_coop_grade_and_wave_fields() {
        let detail = json!({
            "id": "job-1",
            "rule": "REGULAR",
            "coopStage": { "name": "Gone Fission Hydroplant" },
            "resultWave": 4,
            "jobPoint": 120,
            "afterGrade": { "name": "Eggsecutive VP" },
            "afterGradePoint": 80,
            "dangerRate": 0.333,
            "myResult": { "goldenEggs": 12, "powerEggs": 850, "rescued": 2, "rescuedByOthers": 1 }
        });
        let record = parse_coop(&detail, "fallback").unwrap();
        assert_eq!(record.clear_waves, Some(3));
        assert_eq!(record.golden_eggs, Some(12));
        assert_eq!(record.after_grade.as_deref(), Some("Eggsecutive VP"));
    }

    #[test]
    fn cache_summary_reports_both_history_types() {
        let cache = HistoryCache {
            schema_version: 1,
            fetched_at_ms: 99,
            battles: vec![],
            coop: vec![],
        };
        assert_eq!(cache.summary().battle_count, 0);
        assert_eq!(cache.summary().added_battle_count, 0);
        assert_eq!(
            crate::splatnet_auth::WEB_VIEW_VERSION_FALLBACK,
            "10.0.0-88706e32"
        );
    }

    #[test]
    fn uses_current_persisted_query_ids_for_history_details() {
        assert_eq!(
            X_BATTLE_HISTORIES_QUERY,
            "eb5996a12705c2e94813a62e05c0dc419aad2811b8d49d53e5732290105559cb"
        );
        assert_eq!(
            BATTLE_DETAIL_QUERY,
            "94faa2ff992222d11ced55e0f349920a82ac50f414ae33c83d1d1c9d8161c5dd"
        );
        assert_eq!(
            COOP_QUERY,
            "e11a8cf2c3de7348495dea5cdcaa25e0c153541c4ed63f044b6c174bc5b703df"
        );
        assert_eq!(
            COOP_DETAIL_QUERY,
            "f2d55873a9281213ae27edc171e2b19131b3021a2ae263757543cdd3bf015cc8"
        );
    }

    #[test]
    fn incremental_sync_skips_details_already_in_archive() {
        let known = HashSet::from(["battle-1".to_owned(), "battle-3".to_owned()]);
        let ids = vec![
            "battle-1".to_owned(),
            "battle-2".to_owned(),
            "battle-3".to_owned(),
        ];

        assert_eq!(new_ids(ids, &known), vec!["battle-2"]);
    }

    #[test]
    fn error_diagnostics_redact_credentials_and_omit_unstructured_bodies() {
        let body = serde_json::to_vec(&json!({
            "error": "unauthorized",
            "extensions": {
                "session_token_code": "session-secret",
                "bulletToken": "bullet-secret",
                "authorization": "Bearer access-secret",
                "f": "f-secret",
                "message": "provider echoed session_token_code=private"
            }
        }))
        .unwrap();

        let preview = response_preview(&body);
        assert!(preview.contains("unauthorized"));
        assert!(!preview.contains("session-secret"));
        assert!(!preview.contains("bullet-secret"));
        assert!(!preview.contains("access-secret"));
        assert!(!preview.contains("f-secret"));
        assert!(!preview.contains("private"));

        let plain_text = response_preview(b"provider echoed session-secret");
        assert!(plain_text.contains("non-JSON response omitted"));
        assert!(!plain_text.contains("session-secret"));
    }

    #[test]
    fn graphql_response_parser_classifies_fixture_responses_without_network() {
        let ok = parse_graphql_response(
            reqwest::StatusCode::OK,
            "application/json",
            "req-1",
            br#"{"data":{"history":{"records":[]}}}"#,
            0,
        )
        .unwrap();
        assert!(ok.pointer("/data/history/records").is_some());

        let expired = parse_graphql_response(
            reqwest::StatusCode::UNAUTHORIZED,
            "application/json",
            "req-2",
            br#"{"message":"unauthorized"}"#,
            0,
        )
        .unwrap_err();
        assert_eq!(expired.kind(), HistoryErrorKind::TokenExpired);

        let unauthorized = parse_graphql_response(
            reqwest::StatusCode::OK,
            "application/json",
            "req-3",
            br#"{"errors":[{"message":"expired","extensions":{"code":"UNAUTHENTICATED"}}]}"#,
            0,
        )
        .unwrap_err();
        assert_eq!(unauthorized.kind(), HistoryErrorKind::TokenExpired);

        let throttled = parse_graphql_response(
            reqwest::StatusCode::TOO_MANY_REQUESTS,
            "application/json",
            "req-4",
            br#"{"token":"must-not-appear"}"#,
            3,
        )
        .unwrap_err();
        assert!(throttled.to_string().contains("HTTP 429"));
        assert!(!throttled.to_string().contains("must-not-appear"));
    }

    #[test]
    fn retries_only_transient_statuses_with_exponential_backoff() {
        assert!(is_retryable_status(reqwest::StatusCode::TOO_MANY_REQUESTS));
        assert!(is_retryable_status(reqwest::StatusCode::BAD_GATEWAY));
        assert!(!is_retryable_status(reqwest::StatusCode::BAD_REQUEST));
        assert_eq!(retry_delay(1, None), Duration::from_millis(500));
        assert_eq!(retry_delay(2, None), Duration::from_secs(1));
        assert_eq!(retry_delay(3, None), Duration::from_secs(2));
        assert_eq!(
            retry_delay(1, Some(Duration::from_secs(45))),
            MAX_RETRY_DELAY
        );
    }

    #[test]
    fn merge_deduplicates_overlapping_server_windows_and_keeps_raw_data() {
        let raw = json!({ "id": "battle-1", "newField": true });
        let mut archive = HistoryCache {
            schema_version: 1,
            fetched_at_ms: 1,
            battles: vec![ArchivedBattle {
                record: BattleRecord {
                    id: "battle-1".to_owned(),
                    played_time: Some("2026-09-20T00:00:00Z".to_owned()),
                    mode: "X_MATCH".to_owned(),
                    bankara_mode: None,
                    rule: None,
                    stage: None,
                    judgement: None,
                    player_name: None,
                    weapon: None,
                    paint: None,
                    power: None,
                    power_kind: None,
                    kills: None,
                    assists: None,
                    deaths: None,
                    specials: None,
                },
                raw: json!({ "id": "battle-1" }),
            }],
            coop: vec![],
        };
        let incoming = HistoryCache {
            schema_version: 1,
            fetched_at_ms: 2,
            battles: vec![ArchivedBattle {
                record: archive.battles[0].record.clone(),
                raw: raw.clone(),
            }],
            coop: vec![],
        };
        let (added_battles, _) = archive.merge(incoming);
        assert_eq!(added_battles, 0);
        assert_eq!(archive.battles.len(), 1);
        assert_eq!(archive.battles[0].raw, raw);
    }
}
