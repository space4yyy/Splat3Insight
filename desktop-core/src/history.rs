//! SplatNet 3 history client and the compact records used by the overlay.
//!
//! The client deliberately talks to SplatNet directly. It does not read the
//! s3s export directory. It keeps one raw detail response per stable SplatNet
//! detail id together with a compact parsed record, so old data can be
//! re-parsed later without storing duplicate responses on every sync.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

use crate::splatnet_auth::{fetch_web_view_version, SPLATNET_URL};

const LATEST_BATTLE_QUERY: &str =
    "b24d22fd6cb251c515c2b90044039698aa27bc1fab15801d83014d919cd45780";
const BATTLE_DETAIL_QUERY: &str =
    "94faa2ff992222d11ced55e0f349920a82ac50f414ae33c83d1d1c9d8161c5dd";
const COOP_QUERY: &str =
    "e11a8cf2c3de7348495dea5cdcaa25e0c153541c4ed63f044b6c174bc5b703df";
const COOP_DETAIL_QUERY: &str =
    "f2d55873a9281213ae27edc171e2b19131b3021a2ae263757543cdd3bf015cc8";

const APP_USER_AGENT: &str = "Mozilla/5.0 (Linux; Android 14; Pixel 7a) AppleWebKit/537.36 Chrome/120.0.6099.230 Mobile Safari/537.36";

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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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
        self.battles.sort_by(|left, right| {
            right
                .record
                .played_time
                .cmp(&left.record.played_time)
        });
        self.coop.sort_by(|left, right| {
            right
                .record
                .played_time
                .cmp(&left.record.played_time)
        });
        (added_battles, added_coop)
    }
}

pub struct SplatNetDataClient {
    http: reqwest::Client,
    config: SplatNetDataConfig,
    web_view_version: String,
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
            _ => fetch_web_view_version(
                &http,
                &config.game_web_token,
                &config.language,
                &config.country,
                &config.app_user_agent,
            )
            .await,
        };

        Ok(Self {
            http,
            config,
            web_view_version,
        })
    }

    pub async fn fetch_history(
        &self,
        battle_limit: usize,
        coop_limit: usize,
        fetched_at_ms: i64,
    ) -> Result<HistoryCache, String> {
        let battle_ids = self
            .fetch_ids(LATEST_BATTLE_QUERY, "latestBattleHistories", battle_limit, None)
            .await
            .map_err(|error| format!("对战列表查询失败：{error}"))?;
        let coop_ids = self
            .fetch_ids(COOP_QUERY, "coopResult", coop_limit, Some("en-US"))
            .await
            .map_err(|error| format!("打工列表查询失败：{error}"))?;

        let mut battles = Vec::with_capacity(battle_ids.len());
        for id in battle_ids {
            if let Some(detail) = self
                .fetch_detail(BATTLE_DETAIL_QUERY, "vsResultId", &id, None)
                .await
                .map_err(|error| format!("对战详情查询失败（第 {} 条）：{error}", battles.len() + 1))?
            {
                if let Some(record) = parse_battle(&detail, &id) {
                    battles.push(ArchivedBattle { record, raw: detail });
                }
            }
        }

        let mut coop = Vec::with_capacity(coop_ids.len());
        for id in coop_ids {
            if let Some(detail) = self
                .fetch_detail(COOP_DETAIL_QUERY, "coopHistoryDetailId", &id, Some("en-US"))
                .await
                .map_err(|error| format!("打工详情查询失败（第 {} 条）：{error}", coop.len() + 1))?
            {
                if let Some(record) = parse_coop(&detail, &id) {
                    coop.push(ArchivedCoop { record, raw: detail });
                }
            }
        }

        battles.sort_by(|left, right| {
            right
                .record
                .played_time
                .cmp(&left.record.played_time)
        });
        coop.sort_by(|left, right| {
            right
                .record
                .played_time
                .cmp(&left.record.played_time)
        });

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
    ) -> Result<Vec<String>, String> {
        let value = self.graphql(query_hash, json!({}), language).await?;
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
    ) -> Result<Option<Value>, String> {
        let value = self
            .graphql(
                query_hash,
                json!({ variable_name: id }),
                language,
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

    async fn graphql(
        &self,
        query_hash: &str,
        variables: Value,
        language: Option<&str>,
    ) -> Result<Value, String> {
        let language = language.unwrap_or(&self.config.language);
        let response = self
            .http
            .post(format!("{SPLATNET_URL}/api/graphql"))
            .header("Authorization", format!("Bearer {}", self.config.bullet_token))
            .header("Accept-Language", language)
            .header("User-Agent", &self.config.app_user_agent)
            .header("X-Web-View-Ver", &self.web_view_version)
            .header("Content-Type", "application/json")
            .header("Accept", "*/*")
            .header("Origin", SPLATNET_URL)
            .header("X-Requested-With", "com.nintendo.znca")
            .header("Referer", format!("{SPLATNET_URL}?lang={language}&na_country={}&na_lang={language}", self.config.country))
            .header("Cookie", format!("_gtoken={}; _dnt=1", self.config.game_web_token))
            .json(&json!({
                "extensions": { "persistedQuery": { "sha256Hash": query_hash, "version": 1 } },
                "variables": variables,
            }))
            .send()
            .await
            .map_err(|error| format!("SplatNet 数据请求失败：{error}"))?;
        let status = response.status();
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
        let response_body = response
            .bytes()
            .await
            .map_err(|error| format!("读取 SplatNet 数据响应失败（HTTP {status}）：{error}"))?;
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err("SplatNet bullet token 已失效，请重新获取 SplatNet 权限".to_owned());
        }
        if status == reqwest::StatusCode::FORBIDDEN {
            return Err("SplatNet 拒绝了数据请求，客户端版本或 Web View 版本可能已过期".to_owned());
        }
        if !status.is_success() {
            return Err(format!(
                "SplatNet 数据请求失败（HTTP {status}，Content-Type: {content_type}，Request-ID: {request_id}，响应：{}）",
                response_preview(&response_body)
            ));
        }
        let value: Value = serde_json::from_slice(&response_body).map_err(|error| {
            format!(
                "SplatNet 数据响应不是有效 JSON（HTTP {status}，Content-Type: {content_type}，Request-ID: {request_id}，响应：{}）：{error}",
                response_preview(&response_body)
            )
        })?;
        if let Some(errors) = value.get("errors").and_then(Value::as_array) {
            if let Some(first) = errors.first() {
                return Err(format!("SplatNet GraphQL 返回错误：{}", compact_json(first)));
            }
        }
        if value.get("data").is_none() {
            return Err("SplatNet 响应缺少 data".to_owned());
        }
        Ok(value)
    }
}

fn parse_battle(detail: &Value, fallback_id: &str) -> Option<BattleRecord> {
    let id = string_at(detail, &["id"]).unwrap_or_else(|| fallback_id.to_owned());
    let mode = string_at(detail, &["vsMode", "mode"]).unwrap_or_else(|| "BATTLE".to_owned());
    let bankara_mode = string_at(detail, &["bankaraMatch", "mode"]);
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
                integer_at(value, &["goldenEgg"])
                    .or_else(|| integer_at(value, &["goldenEggs"])),
                integer_at(value, &["powerEgg"])
                    .or_else(|| integer_at(value, &["powerEggs"])),
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
    serde_json::to_string(value).unwrap_or_else(|_| "未知 GraphQL 错误".to_owned())
}

fn response_preview(body: &[u8]) -> String {
    let text = String::from_utf8_lossy(body);
    let preview: String = text.chars().take(240).collect();
    let preview = preview.replace(['\n', '\r', '\t'], " ");
    if text.chars().count() > 240 {
        format!("{preview}…")
    } else if preview.is_empty() {
        format!("<empty body, {} bytes>", body.len())
    } else {
        preview
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn parses_bankara_open_and_challenge_power_shapes() {
        let open = json!({
            "vsMode": { "mode": "BANKARA" },
            "bankaraMatch": { "mode": "OPEN", "bankaraPower": { "power": 1878.3 } }
        });
        let challenge = json!({
            "vsMode": { "mode": "BANKARA" },
            "bankaraMatch": { "mode": "CHALLENGE", "weaponPower": { "power": 2019.8 } }
        });
        assert_eq!(parse_battle(&open, "open").unwrap().power_kind.as_deref(), Some("team"));
        assert_eq!(parse_battle(&challenge, "challenge").unwrap().power, Some(2019.8));
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
        assert_eq!(crate::splatnet_auth::WEB_VIEW_VERSION_FALLBACK, "10.0.0-88706e32");
    }

    #[test]
    fn uses_current_persisted_query_ids_for_history_details() {
        assert_eq!(BATTLE_DETAIL_QUERY, "94faa2ff992222d11ced55e0f349920a82ac50f414ae33c83d1d1c9d8161c5dd");
        assert_eq!(COOP_QUERY, "e11a8cf2c3de7348495dea5cdcaa25e0c153541c4ed63f044b6c174bc5b703df");
        assert_eq!(COOP_DETAIL_QUERY, "f2d55873a9281213ae27edc171e2b19131b3021a2ae263757543cdd3bf015cc8");
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
