use crate::dashboard_store;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use rusqlite::{params, Connection, OptionalExtension};
use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
#[cfg(test)]
use splat3_widget_core::history::ArchivedCoop;
use splat3_widget_core::history::{
    parse_battle, ArchivedBattle, BattleRecord, CoopRecord, HistoryCache, HistoryFetchedRecord,
    HistorySyncSummary,
};
use splat3_widget_core::overlay::{
    archived_battle_power_track_key, OverlayMatchCategory, OverlayState, RecentSummary,
    SalmonRunCard,
};
use std::collections::HashSet;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const DATABASE_FILE: &str = "history.sqlite3";
const LEGACY_DIRECTORY: &str = "legacy-json";
const EXPORT_DIRECTORY: &str = "exports";
/// 导出文件的格式版本，和 `write_complete_archive` 写出的一致。
const EXPORT_SCHEMA_VERSION: i64 = 2;
/// 导入时每批提交的记录数，不长时间占住数据库，后台同步照常进行。
const IMPORT_BATCH_SIZE: usize = 100;
const DATABASE_SCHEMA_VERSION: i64 = 2;
const NO_OVERLAY_TRACK: &str = "none";
const CACHED_POWER_TRACK_KEY: &str = "__s3w_overlay_power_track_v1";
const EVENT_POWER_MIGRATION: &str = "event-power-fields-v1";
const MODE_DELTA_MIGRATION: &str = "mode-level-deltas-v1";
const RATING_RUN_MIGRATION: &str = "rating-run-deltas-v1";
/// 卡片数据的升级：缺任何一项时，重新解析活动比赛记录并重建卡片缓存，然后全部标记完成。
const OVERLAY_MIGRATIONS: [&str; 3] = [
    EVENT_POWER_MIGRATION,
    MODE_DELTA_MIGRATION,
    RATING_RUN_MIGRATION,
];
/// 一般打工每期 40 小时。往前找同一期工时只看这个范围，既能覆盖整期，
/// 又不会把隔了几周、恰好场地和武器都相同的另一期算进来。
const COOP_ROTATION_MAX_HOURS: i64 = 48;

fn database_path(history_dir: &Path) -> PathBuf {
    history_dir.join(DATABASE_FILE)
}

fn legacy_path(history_dir: &Path, account_id: &str) -> PathBuf {
    history_dir.join(format!("{account_id}.json"))
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or_default()
}

pub(crate) fn open_database(history_dir: &Path) -> Result<Connection, String> {
    fs::create_dir_all(history_dir).map_err(|error| format!("创建历史数据目录失败：{error}"))?;
    let connection = Connection::open(database_path(history_dir))
        .map_err(|error| format!("打开历史数据库失败：{error}"))?;
    connection
        .busy_timeout(std::time::Duration::from_secs(5))
        .map_err(|error| format!("设置历史数据库锁等待失败：{error}"))?;
    connection
        .execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             CREATE TABLE IF NOT EXISTS history_records (
                 account_id TEXT NOT NULL,
                 record_type TEXT NOT NULL CHECK(record_type IN ('battle', 'coop')),
                 record_id TEXT NOT NULL,
                 played_time TEXT,
                 record_json TEXT NOT NULL,
                 raw_json BLOB NOT NULL,
                 raw_encoding TEXT NOT NULL DEFAULT 'gzip',
                 overlay_track_key TEXT,
                 PRIMARY KEY (account_id, record_type, record_id)
             );
             CREATE INDEX IF NOT EXISTS idx_history_records_time
                 ON history_records(account_id, record_type, played_time DESC);
             CREATE TABLE IF NOT EXISTS history_meta (
                 account_id TEXT PRIMARY KEY,
                 schema_version INTEGER NOT NULL,
                 fetched_at_ms INTEGER NOT NULL,
                 sync_complete INTEGER NOT NULL DEFAULT 1
             );
             CREATE TABLE IF NOT EXISTS overlay_snapshots (
                 account_id TEXT PRIMARY KEY,
                 state_json TEXT NOT NULL,
                 updated_at_ms INTEGER NOT NULL
             );
             CREATE TABLE IF NOT EXISTS history_migrations (
                 account_id TEXT NOT NULL,
                 migration_key TEXT NOT NULL,
                 version INTEGER NOT NULL,
                 PRIMARY KEY (account_id, migration_key)
             );
             CREATE TABLE IF NOT EXISTS x_battle_measurement_snapshots (
                 account_id TEXT NOT NULL,
                 record_id TEXT NOT NULL,
                 payload_hash TEXT NOT NULL,
                 fetched_at_ms INTEGER NOT NULL,
                 raw_json BLOB NOT NULL,
                 raw_encoding TEXT NOT NULL DEFAULT 'gzip',
                 PRIMARY KEY (account_id, record_id, payload_hash)
             );",
        )
        .map_err(|error| format!("初始化历史数据库失败：{error}"))?;
    let has_sync_complete: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('history_meta')
             WHERE name = 'sync_complete'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| format!("检查历史数据库版本失败：{error}"))?;
    if has_sync_complete == 0 {
        connection
            .execute_batch(
                "ALTER TABLE history_meta
                 ADD COLUMN sync_complete INTEGER NOT NULL DEFAULT 1;",
            )
            .map_err(|error| format!("升级历史数据库失败：{error}"))?;
    }
    let has_overlay_track_key: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM pragma_table_info('history_records')
             WHERE name = 'overlay_track_key'",
            [],
            |row| row.get(0),
        )
        .map_err(|error| format!("检查卡片历史索引版本失败：{error}"))?;
    if has_overlay_track_key == 0 {
        connection
            .execute_batch("ALTER TABLE history_records ADD COLUMN overlay_track_key TEXT;")
            .map_err(|error| format!("升级卡片历史索引失败：{error}"))?;
    }
    connection
        .execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_history_records_overlay_track
                 ON history_records(account_id, overlay_track_key, played_time DESC);",
        )
        .map_err(|error| format!("创建卡片历史索引失败：{error}"))?;
    dashboard_store::ensure_schema(&connection)?;
    Ok(connection)
}

pub(crate) fn check_integrity(history_dir: &Path) -> Result<(), String> {
    let path = database_path(history_dir);
    if !path.exists() {
        return Ok(());
    }
    let connection = Connection::open(path)
        .map_err(|error| format!("打开历史数据库进行完整性检查失败：{error}"))?;
    let result: String = connection
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .map_err(|error| format!("执行历史数据库完整性检查失败：{error}"))?;
    if result == "ok" {
        Ok(())
    } else {
        Err(format!("历史数据库完整性检查失败：{result}"))
    }
}

pub(crate) fn needs_full_sync(history_dir: &Path, account_id: &str) -> Result<bool, String> {
    let connection = open_database(history_dir)?;
    let complete = connection
        .query_row(
            "SELECT sync_complete FROM history_meta WHERE account_id = ?1",
            [account_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(|error| format!("读取历史同步断点状态失败：{error}"))?;
    Ok(complete != Some(1))
}

pub(crate) fn mark_sync_started(
    history_dir: &Path,
    account_id: &str,
    fetched_at_ms: i64,
) -> Result<(), String> {
    let connection = open_database(history_dir)?;
    connection
        .execute(
            "INSERT INTO history_meta(account_id, schema_version, fetched_at_ms, sync_complete)
             VALUES (?1, ?2, ?3, 0)
             ON CONFLICT(account_id) DO UPDATE SET
                 schema_version = excluded.schema_version,
                 fetched_at_ms = CASE
                     WHEN history_meta.sync_complete = 1 THEN history_meta.fetched_at_ms
                     ELSE excluded.fetched_at_ms
                 END,
                 sync_complete = history_meta.sync_complete",
            params![account_id, DATABASE_SCHEMA_VERSION, fetched_at_ms],
        )
        .map_err(|error| format!("保存历史同步开始状态失败：{error}"))?;
    Ok(())
}

pub(crate) fn mark_sync_complete(
    history_dir: &Path,
    account_id: &str,
    fetched_at_ms: i64,
) -> Result<(), String> {
    let connection = open_database(history_dir)?;
    connection
        .execute(
            "INSERT INTO history_meta(account_id, schema_version, fetched_at_ms, sync_complete)
             VALUES (?1, ?2, ?3, 1)
             ON CONFLICT(account_id) DO UPDATE SET
                 schema_version = excluded.schema_version,
                 fetched_at_ms = excluded.fetched_at_ms,
                 sync_complete = 1",
            params![account_id, DATABASE_SCHEMA_VERSION, fetched_at_ms],
        )
        .map_err(|error| format!("保存历史同步完成状态失败：{error}"))?;
    Ok(())
}

#[cfg(test)]
pub(crate) fn load_cache(
    history_dir: &Path,
    account_id: &str,
) -> Result<Option<HistoryCache>, String> {
    let connection = open_database(history_dir)?;
    let record_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM history_records WHERE account_id = ?1",
            [account_id],
            |row| row.get(0),
        )
        .map_err(|error| format!("读取历史数据库失败：{error}"))?;

    if record_count == 0 {
        return Ok(None);
    }

    let fetched_at_ms = connection
        .query_row(
            "SELECT fetched_at_ms FROM history_meta WHERE account_id = ?1",
            [account_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(|error| format!("读取历史同步时间失败：{error}"))?
        .unwrap_or(0);
    let mut statement = connection
        .prepare(
            "SELECT record_type, record_json, raw_json, raw_encoding
             FROM history_records
             WHERE account_id = ?1
             ORDER BY played_time IS NULL, played_time DESC, rowid DESC",
        )
        .map_err(|error| format!("准备历史记录查询失败：{error}"))?;
    let rows = statement
        .query_map([account_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Vec<u8>>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(|error| format!("读取历史记录失败：{error}"))?;
    let mut battles = Vec::new();
    let mut coop = Vec::new();
    for row in rows {
        let (record_type, record_json, raw_json, raw_encoding) =
            row.map_err(|error| format!("读取历史记录行失败：{error}"))?;
        let raw = decode_raw_json(&raw_json, &raw_encoding)?;
        match record_type.as_str() {
            "battle" => battles.push(ArchivedBattle {
                record: serde_json::from_str::<BattleRecord>(&record_json)
                    .map_err(|error| format!("解析对战记录失败：{error}"))?,
                raw,
            }),
            "coop" => coop.push(ArchivedCoop {
                record: serde_json::from_str::<CoopRecord>(&record_json)
                    .map_err(|error| format!("解析打工记录失败：{error}"))?,
                raw,
            }),
            other => return Err(format!("历史数据库包含未知记录类型：{other}")),
        }
    }

    Ok(Some(HistoryCache {
        schema_version: DATABASE_SCHEMA_VERSION as u32,
        fetched_at_ms,
        battles,
        coop,
    }))
}

pub(crate) fn load_known_record_ids(
    history_dir: &Path,
    account_id: &str,
    limit_per_type: Option<usize>,
) -> Result<(HashSet<String>, HashSet<String>), String> {
    let connection = open_database(history_dir)?;
    let load_ids = |record_type: &str| -> Result<HashSet<String>, String> {
        let sql = if limit_per_type.is_some() {
            "SELECT record_id FROM history_records
             WHERE account_id = ?1 AND record_type = ?2
             ORDER BY played_time IS NULL, played_time DESC, rowid DESC LIMIT ?3"
        } else {
            "SELECT record_id FROM history_records
             WHERE account_id = ?1 AND record_type = ?2"
        };
        let mut statement = connection
            .prepare(sql)
            .map_err(|error| format!("准备历史去重查询失败：{error}"))?;
        match limit_per_type {
            Some(limit) => {
                let rows = statement
                    .query_map(params![account_id, record_type, limit as i64], |row| {
                        row.get::<_, String>(0)
                    })
                    .map_err(|error| format!("读取历史去重信息失败：{error}"))?;
                rows.collect::<Result<HashSet<_>, _>>()
                    .map_err(|error| format!("读取历史去重行失败：{error}"))
            }
            None => {
                let rows = statement
                    .query_map(params![account_id, record_type], |row| {
                        row.get::<_, String>(0)
                    })
                    .map_err(|error| format!("读取历史去重信息失败：{error}"))?;
                rows.collect::<Result<HashSet<_>, _>>()
                    .map_err(|error| format!("读取历史去重行失败：{error}"))
            }
        }
    };
    Ok((load_ids("battle")?, load_ids("coop")?))
}

pub(crate) fn load_summary(
    history_dir: &Path,
    account_id: &str,
) -> Result<Option<HistorySyncSummary>, String> {
    let connection = open_database(history_dir)?;
    let counts = connection
        .query_row(
            "SELECT
                 SUM(CASE WHEN record_type = 'battle' THEN 1 ELSE 0 END),
                 SUM(CASE WHEN record_type = 'coop' THEN 1 ELSE 0 END)
             FROM history_records WHERE account_id = ?1",
            [account_id],
            |row| Ok((row.get::<_, Option<i64>>(0)?, row.get::<_, Option<i64>>(1)?)),
        )
        .map_err(|error| format!("读取历史统计失败：{error}"))?;
    if counts.0.unwrap_or(0) == 0 && counts.1.unwrap_or(0) == 0 {
        return Ok(None);
    }
    let fetched_at_ms = connection
        .query_row(
            "SELECT fetched_at_ms FROM history_meta WHERE account_id = ?1",
            [account_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(|error| format!("读取历史同步时间失败：{error}"))?
        .unwrap_or(0);
    Ok(Some(HistorySyncSummary {
        battle_count: counts.0.unwrap_or(0) as usize,
        coop_count: counts.1.unwrap_or(0) as usize,
        fetched_at_ms,
        added_battle_count: 0,
        added_coop_count: 0,
    }))
}

pub(crate) fn load_overlay_cache(
    history_dir: &Path,
    account_id: &str,
) -> Result<Option<HistoryCache>, String> {
    let connection = open_database(history_dir)?;
    let records = {
        let mut statement = connection
            .prepare(
                "SELECT record_id, record_json, overlay_track_key FROM history_records
                 WHERE account_id = ?1 AND record_type = 'battle'
                 ORDER BY played_time IS NULL, played_time DESC, rowid DESC",
            )
            .map_err(|error| format!("准备卡片历史查询失败：{error}"))?;
        let rows = statement
            .query_map([account_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            })
            .map_err(|error| format!("读取卡片历史失败：{error}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("读取卡片历史行失败：{error}"))?
    };
    if records.is_empty() {
        return Ok(None);
    }
    let fetched_at_ms = connection
        .query_row(
            "SELECT fetched_at_ms FROM history_meta WHERE account_id = ?1",
            [account_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(|error| format!("读取卡片同步时间失败：{error}"))?
        .unwrap_or(0);
    let mut battles = Vec::new();
    for (record_id, record_json, stored_track_key) in records {
        let record = serde_json::from_str::<BattleRecord>(&record_json)
            .map_err(|error| format!("解析卡片对战记录失败：{error}"))?;
        if !matches!(
            record.mode.as_str(),
            "X_MATCH" | "BANKARA" | "EVENT_MATCH" | "PRIVATE_BATTLE"
        ) {
            continue;
        }
        let track_key = match stored_track_key {
            Some(track_key) => track_key,
            None => {
                let (raw_json, raw_encoding): (Vec<u8>, String) = connection
                    .query_row(
                        "SELECT raw_json, raw_encoding FROM history_records
                         WHERE account_id = ?1 AND record_type = 'battle' AND record_id = ?2",
                        params![account_id, record_id],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .map_err(|error| format!("读取旧版卡片原始记录失败：{error}"))?;
                let raw = decode_raw_json(&raw_json, &raw_encoding)?;
                let track_key = archived_battle_power_track_key(&record, &raw)
                    .unwrap_or_else(|| NO_OVERLAY_TRACK.to_owned());
                connection
                    .execute(
                        "UPDATE history_records SET overlay_track_key = ?3
                         WHERE account_id = ?1 AND record_type = 'battle' AND record_id = ?2",
                        params![account_id, record_id, track_key],
                    )
                    .map_err(|error| format!("保存卡片历史索引失败：{error}"))?;
                track_key
            }
        };
        battles.push(ArchivedBattle {
            record,
            raw: overlay_cache_raw(&track_key),
        });
    }
    Ok(Some(HistoryCache {
        schema_version: DATABASE_SCHEMA_VERSION as u32,
        fetched_at_ms,
        battles,
        coop: Vec::new(),
    }))
}

fn load_salmon_run_card(
    history_dir: &Path,
    account_id: &str,
) -> Result<Option<SalmonRunCard>, String> {
    let connection = open_database(history_dir)?;
    let mut statement = connection
        .prepare(
            "SELECT played_time, record_json, raw_json, raw_encoding
             FROM history_records
             WHERE account_id = ?1 AND record_type = 'coop'
             ORDER BY played_time IS NULL, played_time DESC, rowid DESC",
        )
        .map_err(|error| format!("准备鲑鱼跑卡片查询失败：{error}"))?;
    let mut rows = statement
        .query_map(params![account_id], |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Vec<u8>>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(|error| format!("读取鲑鱼跑卡片失败：{error}"))?;
    let mut latest = None;
    for row in rows.by_ref() {
        let (played_time, record_json, raw_bytes, encoding) =
            row.map_err(|error| format!("读取鲑鱼跑卡片行失败：{error}"))?;
        let record: Value = match serde_json::from_str(&record_json) {
            Ok(record) => record,
            Err(_) => continue,
        };
        if record.get("rule").and_then(Value::as_str) != Some("REGULAR") {
            continue;
        }
        latest = Some(((played_time, raw_bytes, encoding), record));
        break;
    }
    let Some(((latest_played_time, latest_raw_bytes, latest_encoding), latest_record)) = latest
    else {
        return Ok(None);
    };
    let latest_raw = decode_raw_json(&latest_raw_bytes, &latest_encoding).unwrap_or(Value::Null);
    let boss_enemy_id = latest_raw
        .pointer("/boss/coopEnemyId")
        .and_then(Value::as_i64)
        .filter(|id| matches!(id, 23 | 24 | 25 | 30));
    let boss_progress = boss_enemy_id.and_then(|_| {
        if latest_raw
            .pointer("/bossResult/hasDefeatBoss")
            .and_then(Value::as_bool)
            == Some(true)
        {
            Some(0)
        } else {
            latest_raw
                .get("smellMeter")
                .and_then(Value::as_i64)
                .map(|value| value.saturating_add(1).clamp(1, 5))
        }
    });
    let stage = latest_record
        .get("stage")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let latest_time = latest_played_time
        .as_deref()
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok());
    let rotation_key = coop_rotation_key(&latest_record, &latest_raw);

    let mut gold_scale = 0;
    let mut silver_scale = 0;
    let mut bronze_scale = 0;
    let mut has_gold_scale = false;
    let mut has_silver_scale = false;
    let mut has_bronze_scale = false;
    let mut clear_count = 0;
    let mut failure_count = 0;
    let mut boss_encounter_count = 0;
    let mut boss_undefeated_count = 0;

    let mut accumulate_record = |record: &Value, raw: &Value| {
        let clear_waves = record.get("clear_waves").and_then(Value::as_i64);
        if let Some(clear_waves) = clear_waves {
            if clear_waves >= 3 {
                clear_count += 1;
            } else {
                failure_count += 1;
            }
        }

        if let Some(scale) = raw.get("scale") {
            add_scale(scale, "gold", &mut gold_scale, &mut has_gold_scale);
            add_scale(scale, "silver", &mut silver_scale, &mut has_silver_scale);
            add_scale(scale, "bronze", &mut bronze_scale, &mut has_bronze_scale);
        }
        if let Some(boss_enemy_id) = boss_enemy_id {
            let matching_boss =
                raw.pointer("/boss/coopEnemyId").and_then(Value::as_i64) == Some(boss_enemy_id);
            if matching_boss {
                if let Some(boss_result) = raw.get("bossResult").filter(|value| value.is_object()) {
                    boss_encounter_count += 1;
                    if boss_result.get("hasDefeatBoss").and_then(Value::as_bool) == Some(false) {
                        boss_undefeated_count += 1;
                    }
                }
            }
        }
    };
    accumulate_record(&latest_record, &latest_raw);

    // 同一期工：优先用 SplatNet 打工列表给的这一期起止时间。之前同步、没有起止时间的记录，
    // 按场地和这期发放的武器判断，并且只看一期的时长之内。
    // 中间穿插的大型跑、团队打工或私人打工直接跳过，不会把这一期截断。
    let rotation_window = coop_rotation_window(&latest_record);
    if let Some(latest_time) = latest_time {
        let window_start = rotation_window
            .as_ref()
            .and_then(|(start, _)| chrono::DateTime::parse_from_rfc3339(start).ok())
            .unwrap_or_else(|| latest_time - chrono::Duration::hours(COOP_ROTATION_MAX_HOURS));
        for record_row in rows {
            let (played_time, record_json, raw_bytes, encoding) =
                record_row.map_err(|error| format!("读取鲑鱼跑卡片行失败：{error}"))?;
            let Some(played_at) = played_time
                .as_deref()
                .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
            else {
                continue;
            };
            if played_at < window_start {
                break;
            }
            let Ok(record) = serde_json::from_str::<Value>(&record_json) else {
                continue;
            };
            if record.get("rule").and_then(Value::as_str) != Some("REGULAR") {
                continue;
            }
            let raw = decode_raw_json(&raw_bytes, &encoding).unwrap_or(Value::Null);
            let same_rotation = match (&rotation_window, coop_rotation_window(&record)) {
                (Some(latest_window), Some(window)) => *latest_window == window,
                _ => rotation_key.is_some() && coop_rotation_key(&record, &raw) == rotation_key,
            };
            if same_rotation {
                accumulate_record(&record, &raw);
            }
        }
    }

    let my_result = latest_raw.get("myResult").unwrap_or(&Value::Null);
    let species = latest_raw
        .pointer("/myResult/player/species")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned);
    let latest_scales = latest_raw.get("scale").unwrap_or(&Value::Null);
    let record_i64 = |key: &str| latest_record.get(key).and_then(Value::as_i64);
    let my_result_i64 = |keys: &[&str]| -> Option<i64> {
        keys.iter()
            .find_map(|key| my_result.get(*key).and_then(Value::as_i64))
    };
    let scale_count = |key: &str, accumulated: i64, has_accumulated: bool| {
        has_accumulated
            .then_some(accumulated)
            .or_else(|| latest_scales.get(key).and_then(Value::as_i64))
    };
    let grade_point = latest_record
        .get("after_grade_point")
        .and_then(value_as_i64)
        .or_else(|| latest_raw.get("afterGradePoint").and_then(value_as_i64));
    let danger_rate = latest_record
        .get("danger_rate")
        .and_then(Value::as_f64)
        .or_else(|| latest_raw.get("dangerRate").and_then(Value::as_f64));

    Ok(Some(SalmonRunCard {
        played_time: latest_played_time.clone(),
        stage,
        species,
        boss_enemy_id,
        boss_progress,
        boss_encounter_count: boss_enemy_id.map(|_| boss_encounter_count),
        boss_undefeated_count: boss_enemy_id.map(|_| boss_undefeated_count),
        gold_scale: scale_count("gold", gold_scale, has_gold_scale),
        silver_scale: scale_count("silver", silver_scale, has_silver_scale),
        bronze_scale: scale_count("bronze", bronze_scale, has_bronze_scale),
        defeat_enemy_count: my_result_i64(&["defeatEnemyCount"]),
        golden_egg_count: my_result_i64(&["goldenDeliverCount", "goldenEggs", "goldenEgg"])
            .or_else(|| record_i64("golden_eggs")),
        power_egg_count: my_result_i64(&["deliverCount", "powerEggs", "powerEgg"])
            .or_else(|| record_i64("power_eggs")),
        rescued_count: my_result_i64(&["rescuedCount", "rescued"])
            .or_else(|| record_i64("rescued")),
        rescue_count: my_result_i64(&["rescueCount", "rescuedByOthers"])
            .or_else(|| record_i64("rescued_by_others")),
        clear_count,
        failure_count,
        grade: latest_record
            .get("after_grade")
            .and_then(Value::as_str)
            .map(ToOwned::to_owned)
            .or_else(|| {
                latest_raw
                    .pointer("/afterGrade/name")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned)
            }),
        grade_point,
        danger_rate,
    }))
}

/// SplatNet 打工列表给的这一期起止时间，同步时写进记录。
fn coop_rotation_window(record: &Value) -> Option<(String, String)> {
    Some((
        record.get("rotation_start_time")?.as_str()?.to_owned(),
        record.get("rotation_end_time")?.as_str()?.to_owned(),
    ))
}

/// 给列表里已归档的打工记录补上这一期的起止时间；已经有的不改，原始 JSON 不动。
fn fill_coop_rotation(
    transaction: &rusqlite::Transaction<'_>,
    account_id: &str,
    start_time: &str,
    end_time: &str,
    record_ids: &[String],
) -> Result<(), String> {
    let mut select = transaction
        .prepare(
            "SELECT record_json FROM history_records
             WHERE account_id = ?1 AND record_type = 'coop' AND record_id = ?2",
        )
        .map_err(|error| format!("准备补写打工期时间失败：{error}"))?;
    let mut update = transaction
        .prepare(
            "UPDATE history_records SET record_json = ?3
             WHERE account_id = ?1 AND record_type = 'coop' AND record_id = ?2",
        )
        .map_err(|error| format!("准备补写打工期时间失败：{error}"))?;
    for record_id in record_ids {
        let Some(record_json) = select
            .query_row(params![account_id, record_id], |row| {
                row.get::<_, String>(0)
            })
            .optional()
            .map_err(|error| format!("读取打工记录失败：{error}"))?
        else {
            continue;
        };
        let Ok(Value::Object(mut record)) = serde_json::from_str::<Value>(&record_json) else {
            continue;
        };
        if coop_rotation_window(&Value::Object(record.clone())).is_some() {
            continue;
        }
        record.insert(
            "rotation_start_time".to_owned(),
            Value::String(start_time.to_owned()),
        );
        record.insert(
            "rotation_end_time".to_owned(),
            Value::String(end_time.to_owned()),
        );
        let encoded = serde_json::to_string(&record)
            .map_err(|error| format!("写入打工期时间失败：{error}"))?;
        update
            .execute(params![account_id, record_id, encoded])
            .map_err(|error| format!("写入打工期时间失败：{error}"))?;
        dashboard_store::update_coop_rotation(
            transaction,
            account_id,
            record_id,
            start_time,
            end_time,
        )?;
    }
    Ok(())
}

/// 一期工的标识：场地加上这一期发放的四把武器（随机武器期是问号武器，同样固定）。
/// 打工记录固定按英文拉取，武器名可以直接比较；没有武器信息时只比较场地。
fn coop_rotation_key(record: &Value, raw: &Value) -> Option<String> {
    let stage = raw
        .pointer("/coopStage/id")
        .and_then(Value::as_str)
        .or_else(|| record.get("stage").and_then(Value::as_str))?;
    let weapons = raw
        .get("weapons")
        .and_then(Value::as_array)
        .map(|weapons| {
            weapons
                .iter()
                .filter_map(|weapon| {
                    weapon
                        .get("name")
                        .and_then(Value::as_str)
                        .or_else(|| weapon.pointer("/image/url").and_then(Value::as_str))
                })
                .collect::<Vec<_>>()
                .join("\u{1f}")
        })
        .unwrap_or_default();
    Some(format!("{stage}\u{1e}{weapons}"))
}

fn add_scale(scale: &Value, key: &str, total: &mut i64, found: &mut bool) {
    if let Some(value) = scale.get(key).and_then(Value::as_i64) {
        *total += value;
        *found = true;
    }
}

fn value_as_i64(value: &Value) -> Option<i64> {
    value.as_i64().or_else(|| {
        value
            .as_f64()
            .filter(|number| number.fract() == 0.0)
            .map(|n| n as i64)
    })
}

pub(crate) fn load_overlay_state(
    history_dir: &Path,
    account_id: &str,
) -> Result<OverlayState, String> {
    let connection = open_database(history_dir)?;
    if overlay_migration_pending(&connection, account_id)? {
        drop(connection);
        reparse_event_match_records(history_dir, account_id)?;
        let state = match load_overlay_cache(history_dir, account_id)? {
            Some(cache) => OverlayState::from_history(&cache),
            None => empty_overlay_state(),
        };
        save_overlay_state(history_dir, account_id, &state)?;
        let connection = open_database(history_dir)?;
        for migration in OVERLAY_MIGRATIONS {
            connection
                .execute(
                    "INSERT INTO history_migrations(account_id, migration_key, version)
                     VALUES (?1, ?2, 1)
                     ON CONFLICT(account_id, migration_key) DO UPDATE SET
                         version = excluded.version",
                    params![account_id, migration],
                )
                .map_err(|error| format!("保存卡片数据版本失败：{error}"))?;
        }
        drop(connection);
        return attach_salmon_run_card(history_dir, account_id, state);
    }
    let snapshot = connection
        .query_row(
            "SELECT state_json FROM overlay_snapshots WHERE account_id = ?1",
            [account_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| format!("读取卡片缓存失败：{error}"))?;
    if let Some(snapshot) = snapshot {
        match serde_json::from_str(&snapshot) {
            Ok(state) => {
                drop(connection);
                return attach_salmon_run_card(history_dir, account_id, state);
            }
            Err(_) => {
                connection
                    .execute(
                        "DELETE FROM overlay_snapshots WHERE account_id = ?1",
                        [account_id],
                    )
                    .map_err(|error| format!("清理损坏的卡片缓存失败：{error}"))?;
            }
        }
    }
    drop(connection);

    let state = match load_overlay_cache(history_dir, account_id)? {
        Some(cache) => OverlayState::from_history(&cache),
        None => OverlayState {
            selected: None,
            salmon_run: None,
            recent_summary: RecentSummary {
                sample_size: 0,
                win: 0,
                lose: 0,
                win_rate: 0.0,
            },
        },
    };
    save_overlay_state(history_dir, account_id, &state)?;
    attach_salmon_run_card(history_dir, account_id, state)
}

pub(crate) fn load_overlay_state_for_match_category(
    history_dir: &Path,
    account_id: &str,
    category: OverlayMatchCategory,
) -> Result<OverlayState, String> {
    let current_state = load_overlay_state(history_dir, account_id)?;
    let mut state = match load_overlay_cache(history_dir, account_id)? {
        Some(cache) => OverlayState::from_history_for_match_category(&cache, category),
        None => empty_overlay_state(),
    };
    state.salmon_run = current_state.salmon_run;
    Ok(state)
}

fn overlay_migration_pending(connection: &Connection, account_id: &str) -> Result<bool, String> {
    for migration in OVERLAY_MIGRATIONS {
        let version = connection
            .query_row(
                "SELECT version FROM history_migrations
                 WHERE account_id = ?1 AND migration_key = ?2",
                params![account_id, migration],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|error| format!("读取卡片数据版本失败：{error}"))?;
        if version != Some(1) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn attach_salmon_run_card(
    history_dir: &Path,
    account_id: &str,
    mut state: OverlayState,
) -> Result<OverlayState, String> {
    state.salmon_run = load_salmon_run_card(history_dir, account_id)?;
    Ok(state)
}

fn empty_overlay_state() -> OverlayState {
    OverlayState {
        selected: None,
        salmon_run: None,
        recent_summary: RecentSummary {
            sample_size: 0,
            win: 0,
            lose: 0,
            win_rate: 0.0,
        },
    }
}

fn reparse_event_match_records(history_dir: &Path, account_id: &str) -> Result<(), String> {
    let connection = open_database(history_dir)?;
    let records = {
        let mut statement = connection
            .prepare(
                "SELECT record_id, record_json, raw_json, raw_encoding
                 FROM history_records
                 WHERE account_id = ?1 AND record_type = 'battle'
                   AND record_json LIKE '%\"mode\":\"EVENT_MATCH\"%'",
            )
            .map_err(|error| format!("准备活动赛卡片数据升级失败：{error}"))?;
        let rows = statement
            .query_map([account_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Vec<u8>>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })
            .map_err(|error| format!("读取待升级活动赛记录失败：{error}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("读取待升级活动赛记录行失败：{error}"))?
    };

    for (record_id, record_json, raw_bytes, raw_encoding) in records {
        let mut record = serde_json::from_str::<BattleRecord>(&record_json)
            .map_err(|error| format!("解析待升级活动赛记录失败：{error}"))?;
        if record.mode != "EVENT_MATCH" {
            continue;
        }
        let raw = decode_raw_json(&raw_bytes, &raw_encoding)?;
        refresh_event_power(&mut record, &raw);
        let track_key = archived_battle_power_track_key(&record, &raw)
            .unwrap_or_else(|| NO_OVERLAY_TRACK.to_owned());
        let updated_json = serde_json::to_string(&record)
            .map_err(|error| format!("序列化升级后的活动赛记录失败：{error}"))?;
        connection
            .execute(
                "UPDATE history_records
                 SET record_json = ?3, overlay_track_key = ?4
                 WHERE account_id = ?1 AND record_type = 'battle' AND record_id = ?2",
                params![account_id, record_id, updated_json, track_key],
            )
            .map_err(|error| format!("更新活动赛卡片数据失败：{error}"))?;
    }
    Ok(())
}

/// 活动比赛的战力字段是后来才解析的，旧版本存下的记录可能没有这个值，按原始 JSON 补上。
fn refresh_event_power(record: &mut BattleRecord, raw: &Value) {
    if record.mode != "EVENT_MATCH" {
        return;
    }
    if let Some(parsed) = parse_battle(raw, &record.id) {
        if parsed.power.is_some() {
            record.power = parsed.power;
            record.power_kind = parsed.power_kind;
        }
    }
}

pub(crate) fn save_overlay_state(
    history_dir: &Path,
    account_id: &str,
    state: &OverlayState,
) -> Result<(), String> {
    let state_json =
        serde_json::to_string(state).map_err(|error| format!("序列化卡片缓存失败：{error}"))?;
    let connection = open_database(history_dir)?;
    connection
        .execute(
            "INSERT INTO overlay_snapshots(account_id, state_json, updated_at_ms)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(account_id) DO UPDATE SET
                 state_json = excluded.state_json,
                 updated_at_ms = excluded.updated_at_ms",
            params![account_id, state_json, now_ms()],
        )
        .map_err(|error| format!("保存卡片缓存失败：{error}"))?;
    Ok(())
}

#[cfg(test)]
pub(crate) fn save_cache(
    history_dir: &Path,
    account_id: &str,
    cache: &HistoryCache,
) -> Result<(), String> {
    let mut connection = open_database(history_dir)?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("开启历史数据库事务失败：{error}"))?;

    for battle in &cache.battles {
        let track_key = archived_battle_power_track_key(&battle.record, &battle.raw);
        upsert_record(
            &transaction,
            account_id,
            "battle",
            &battle.record.id,
            battle.record.played_time.as_deref(),
            &battle.record,
            &battle.raw,
            track_key.as_deref().unwrap_or(NO_OVERLAY_TRACK),
        )?;
        dashboard_store::upsert_battle(&transaction, account_id, &battle.record, &battle.raw)?;
    }
    for item in &cache.coop {
        upsert_record(
            &transaction,
            account_id,
            "coop",
            &item.record.id,
            item.record.played_time.as_deref(),
            &item.record,
            &item.raw,
            NO_OVERLAY_TRACK,
        )?;
        dashboard_store::upsert_coop(&transaction, account_id, &item.record, &item.raw)?;
    }
    transaction
        .execute(
            "INSERT INTO history_meta(account_id, schema_version, fetched_at_ms, sync_complete)
             VALUES (?1, ?2, ?3, 1)
             ON CONFLICT(account_id) DO UPDATE SET
                 schema_version = excluded.schema_version,
                 fetched_at_ms = excluded.fetched_at_ms,
                 sync_complete = 1",
            params![account_id, DATABASE_SCHEMA_VERSION, cache.fetched_at_ms],
        )
        .map_err(|error| format!("保存历史同步信息失败：{error}"))?;
    transaction
        .commit()
        .map_err(|error| format!("提交历史数据库事务失败：{error}"))?;
    save_overlay_state(history_dir, account_id, &OverlayState::from_history(cache))
}

pub(crate) struct HistoryRecordWriter {
    connection: Connection,
}

impl HistoryRecordWriter {
    pub(crate) fn open(history_dir: &Path) -> Result<Self, String> {
        Ok(Self {
            connection: open_database(history_dir)?,
        })
    }

    pub(crate) fn save(
        &mut self,
        account_id: &str,
        fetched_at_ms: i64,
        record: HistoryFetchedRecord,
    ) -> Result<bool, String> {
        let transaction = self
            .connection
            .transaction()
            .map_err(|error| format!("开启历史断点事务失败：{error}"))?;
        let overlay_changed = match record {
            HistoryFetchedRecord::Battle(record) => {
                let track_key = archived_battle_power_track_key(&record.record, &record.raw);
                upsert_record(
                    &transaction,
                    account_id,
                    "battle",
                    &record.record.id,
                    record.record.played_time.as_deref(),
                    &record.record,
                    &record.raw,
                    track_key.as_deref().unwrap_or(NO_OVERLAY_TRACK),
                )?;
                dashboard_store::upsert_battle(
                    &transaction,
                    account_id,
                    &record.record,
                    &record.raw,
                )?;
                false
            }
            HistoryFetchedRecord::Coop(record) => {
                upsert_record(
                    &transaction,
                    account_id,
                    "coop",
                    &record.record.id,
                    record.record.played_time.as_deref(),
                    &record.record,
                    &record.raw,
                    NO_OVERLAY_TRACK,
                )?;
                dashboard_store::upsert_coop(
                    &transaction,
                    account_id,
                    &record.record,
                    &record.raw,
                )?;
                false
            }
            HistoryFetchedRecord::XBattleSettlement {
                record_id,
                power_after,
                payload_hash,
                raw_group,
            } => save_x_battle_settlement(
                &transaction,
                account_id,
                fetched_at_ms,
                &record_id,
                power_after,
                &payload_hash,
                &raw_group,
            )?,
            HistoryFetchedRecord::CoopRotation {
                start_time,
                end_time,
                record_ids,
            } => {
                fill_coop_rotation(
                    &transaction,
                    account_id,
                    &start_time,
                    &end_time,
                    &record_ids,
                )?;
                false
            }
        };
        transaction
            .execute(
                "INSERT INTO history_meta(account_id, schema_version, fetched_at_ms, sync_complete)
                 VALUES (?1, ?2, ?3, 0)
                 ON CONFLICT(account_id) DO UPDATE SET
                     fetched_at_ms = excluded.fetched_at_ms,
                     sync_complete = 0",
                params![account_id, DATABASE_SCHEMA_VERSION, fetched_at_ms],
            )
            .map_err(|error| format!("保存历史断点状态失败：{error}"))?;
        transaction
            .commit()
            .map_err(|error| format!("提交历史断点事务失败：{error}"))?;
        Ok(overlay_changed)
    }
}

fn save_x_battle_settlement(
    transaction: &rusqlite::Transaction<'_>,
    account_id: &str,
    fetched_at_ms: i64,
    record_id: &str,
    power_after: f64,
    payload_hash: &str,
    raw_group: &Value,
) -> Result<bool, String> {
    let raw_json = encode_raw_json(raw_group)?;
    transaction
        .execute(
            "INSERT OR IGNORE INTO x_battle_measurement_snapshots(
                 account_id, record_id, payload_hash, fetched_at_ms, raw_json, raw_encoding
             ) VALUES (?1, ?2, ?3, ?4, ?5, 'gzip')",
            params![account_id, record_id, payload_hash, fetched_at_ms, raw_json],
        )
        .map_err(|error| format!("归档 X 赛测定组原始数据失败：{error}"))?;

    let record_json = transaction
        .query_row(
            "SELECT record_json FROM history_records
             WHERE account_id = ?1 AND record_type = 'battle' AND record_id = ?2",
            params![account_id, record_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| format!("读取待更新的 X 赛战绩失败：{error}"))?;
    let Some(record_json) = record_json else {
        return Ok(false);
    };
    let mut record = serde_json::from_str::<BattleRecord>(&record_json)
        .map_err(|error| format!("解析待更新的 X 赛战绩失败：{error}"))?;
    if record.mode != "X_MATCH" || record.power_kind.as_deref() != Some("x") {
        return Ok(false);
    }
    if record.power == Some(power_after) {
        return Ok(false);
    }
    record.power = Some(power_after);
    let updated_json = serde_json::to_string(&record)
        .map_err(|error| format!("序列化已结算的 X 赛战绩失败：{error}"))?;
    transaction
        .execute(
            "UPDATE history_records SET record_json = ?3
             WHERE account_id = ?1 AND record_type = 'battle' AND record_id = ?2",
            params![account_id, record_id, updated_json],
        )
        .map_err(|error| format!("更新 X 赛结算分数失败：{error}"))?;
    dashboard_store::update_battle_power(transaction, account_id, record_id, power_after)?;
    Ok(true)
}

fn upsert_record<T: Serialize>(
    transaction: &rusqlite::Transaction<'_>,
    account_id: &str,
    record_type: &str,
    record_id: &str,
    played_time: Option<&str>,
    record: &T,
    raw: &Value,
    overlay_track_key: &str,
) -> Result<(), String> {
    let record_json =
        serde_json::to_string(record).map_err(|error| format!("序列化历史记录失败：{error}"))?;
    let raw_json = encode_raw_json(raw)?;
    transaction
        .execute(
            "INSERT INTO history_records(
                 account_id, record_type, record_id, played_time,
                 record_json, raw_json, raw_encoding, overlay_track_key
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'gzip', ?7)
             ON CONFLICT(account_id, record_type, record_id) DO UPDATE SET
                 played_time = excluded.played_time,
                 record_json = excluded.record_json,
                 overlay_track_key = excluded.overlay_track_key",
            params![
                account_id,
                record_type,
                record_id,
                played_time,
                record_json,
                raw_json,
                overlay_track_key
            ],
        )
        .map_err(|error| format!("保存历史记录失败：{error}"))?;
    Ok(())
}

pub(crate) fn export(history_dir: &Path, account_id: &str) -> Result<PathBuf, String> {
    let export_dir = history_dir.join(EXPORT_DIRECTORY);
    fs::create_dir_all(&export_dir).map_err(|error| format!("创建导出目录失败：{error}"))?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("读取导出时间失败：{error}"))?
        .as_millis();
    let file_stem = format!("{account_id}-{timestamp}");
    let path = export_dir.join(format!("{file_stem}.json.gz"));
    let partial_path = export_dir.join(format!("{file_stem}.json.gz.partial"));
    if let Err(error) = write_complete_archive(history_dir, account_id, &partial_path) {
        let _ = fs::remove_file(&partial_path);
        return Err(error);
    }
    if let Err(error) = fs::rename(&partial_path, &path) {
        let _ = fs::remove_file(&partial_path);
        return Err(format!("完成历史数据导出失败：{error}"));
    }
    Ok(path)
}

fn write_complete_archive(
    history_dir: &Path,
    account_id: &str,
    destination: &Path,
) -> Result<(), String> {
    let connection = open_database(history_dir)?;
    let total_records: i64 = connection
        .query_row(
            "SELECT
                 (SELECT COUNT(*) FROM history_records WHERE account_id = ?1) +
                 (SELECT COUNT(*) FROM x_battle_measurement_snapshots WHERE account_id = ?1)",
            [account_id],
            |row| row.get(0),
        )
        .map_err(|error| format!("检查可导出的历史数据失败：{error}"))?;
    if total_records == 0 {
        return Err("当前账号还没有可导出的历史数据".to_owned());
    }
    let fetched_at_ms = connection
        .query_row(
            "SELECT fetched_at_ms FROM history_meta WHERE account_id = ?1",
            [account_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(|error| format!("读取历史同步时间失败：{error}"))?
        .unwrap_or(0);

    let file =
        fs::File::create(destination).map_err(|error| format!("创建导出文件失败：{error}"))?;
    let mut encoder = GzEncoder::new(file, Compression::default());
    let account_id_json = serde_json::to_string(account_id)
        .map_err(|error| format!("序列化导出账号信息失败：{error}"))?;
    write!(
        encoder,
        "{{\"schema_version\":2,\"account_id\":{account_id_json},\"fetched_at_ms\":{fetched_at_ms},\"battles\":["
    )
    .map_err(|error| format!("写入导出文件失败：{error}"))?;

    let mut first = true;
    let mut battle_statement = connection
        .prepare(
            "SELECT record_json, raw_json, raw_encoding FROM history_records
             WHERE account_id = ?1 AND record_type = 'battle'
             ORDER BY played_time IS NULL, played_time DESC, rowid DESC",
        )
        .map_err(|error| format!("准备对战数据导出失败：{error}"))?;
    let battle_rows = battle_statement
        .query_map([account_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|error| format!("读取对战数据导出失败：{error}"))?;
    for row in battle_rows {
        let (record_json, raw_json, raw_encoding) =
            row.map_err(|error| format!("读取对战导出记录失败：{error}"))?;
        let record = serde_json::from_str::<Value>(&record_json)
            .map_err(|error| format!("解析对战导出记录失败：{error}"))?;
        let raw = decode_raw_json(&raw_json, &raw_encoding)?;
        if !first {
            encoder
                .write_all(b",")
                .map_err(|error| format!("写入导出文件失败：{error}"))?;
        }
        first = false;
        serde_json::to_writer(&mut encoder, &json!({ "record": record, "raw": raw }))
            .map_err(|error| format!("生成对战导出数据失败：{error}"))?;
    }
    drop(battle_statement);
    encoder
        .write_all(b"],\"coop\":[")
        .map_err(|error| format!("写入导出文件失败：{error}"))?;

    first = true;
    let mut coop_statement = connection
        .prepare(
            "SELECT record_json, raw_json, raw_encoding FROM history_records
             WHERE account_id = ?1 AND record_type = 'coop'
             ORDER BY played_time IS NULL, played_time DESC, rowid DESC",
        )
        .map_err(|error| format!("准备打工数据导出失败：{error}"))?;
    let coop_rows = coop_statement
        .query_map([account_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|error| format!("读取打工数据导出失败：{error}"))?;
    for row in coop_rows {
        let (record_json, raw_json, raw_encoding) =
            row.map_err(|error| format!("读取打工导出记录失败：{error}"))?;
        let record = serde_json::from_str::<Value>(&record_json)
            .map_err(|error| format!("解析打工导出记录失败：{error}"))?;
        let raw = decode_raw_json(&raw_json, &raw_encoding)?;
        if !first {
            encoder
                .write_all(b",")
                .map_err(|error| format!("写入导出文件失败：{error}"))?;
        }
        first = false;
        serde_json::to_writer(&mut encoder, &json!({ "record": record, "raw": raw }))
            .map_err(|error| format!("生成打工导出数据失败：{error}"))?;
    }
    drop(coop_statement);
    encoder
        .write_all(b"],\"x_battle_measurements\":[")
        .map_err(|error| format!("写入导出文件失败：{error}"))?;

    first = true;
    let mut measurement_statement = connection
        .prepare(
            "SELECT record_id, payload_hash, fetched_at_ms, raw_json, raw_encoding
             FROM x_battle_measurement_snapshots WHERE account_id = ?1
             ORDER BY fetched_at_ms, record_id, payload_hash",
        )
        .map_err(|error| format!("准备 X 赛结算数据导出失败：{error}"))?;
    let measurement_rows = measurement_statement
        .query_map([account_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, Vec<u8>>(3)?,
                row.get::<_, String>(4)?,
            ))
        })
        .map_err(|error| format!("读取 X 赛结算数据导出失败：{error}"))?;
    for row in measurement_rows {
        let (record_id, payload_hash, fetched_at, raw_json, raw_encoding) =
            row.map_err(|error| format!("读取 X 赛结算导出记录失败：{error}"))?;
        let raw = decode_raw_json(&raw_json, &raw_encoding)?;
        if !first {
            encoder
                .write_all(b",")
                .map_err(|error| format!("写入导出文件失败：{error}"))?;
        }
        first = false;
        serde_json::to_writer(
            &mut encoder,
            &json!({
                "record_id": record_id,
                "payload_hash": payload_hash,
                "fetched_at_ms": fetched_at,
                "raw": raw,
            }),
        )
        .map_err(|error| format!("生成 X 赛结算导出数据失败：{error}"))?;
    }
    drop(measurement_statement);
    encoder
        .write_all(b"]}")
        .map_err(|error| format!("写入导出文件失败：{error}"))?;
    encoder
        .finish()
        .map_err(|error| format!("完成历史数据导出失败：{error}"))?;
    Ok(())
}

/// 导入的结果：新加入的对战和打工记录数，以及本地已有、没有重复写入的记录数。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub(crate) struct ImportSummary {
    pub(crate) added_battle_count: usize,
    pub(crate) added_coop_count: usize,
    pub(crate) skipped_count: usize,
}

/// 导入本应用导出的历史文件（`export` 写出的 .json.gz，也接受解压后的 JSON）。
///
/// 先完整读一遍，确认文件完整、属于这个账号，再读第二遍写入本地没有的记录；
/// 本地已有的记录保持不变。文件逐条读取，不会整份放进内存。
pub(crate) fn import(
    history_dir: &Path,
    account_id: &str,
    archive: &[u8],
) -> Result<ImportSummary, String> {
    let header = read_archive(archive, &mut ImportCheck)?;
    if header.schema_version.is_none()
        || header.account_id.is_none()
        || !header.has_battles
        || !header.has_coop
    {
        return Err("不是本应用导出的历史文件".to_owned());
    }
    if header.schema_version != Some(EXPORT_SCHEMA_VERSION) {
        return Err("这个导出文件的版本不受支持".to_owned());
    }
    if header.account_id.as_deref() != Some(account_id) {
        return Err("这份文件是另一个账号导出的，没有导入".to_owned());
    }

    let (known_battles, known_coop) = load_known_record_ids(history_dir, account_id, None)?;
    let mut writer = ImportWriter {
        connection: open_database(history_dir)?,
        account_id,
        known_battles,
        known_coop,
        in_batch: 0,
        batch: ImportSummary::default(),
        summary: ImportSummary::default(),
    };
    let written = read_archive(archive, &mut writer).and_then(|_| writer.commit());
    if written.is_err() {
        writer.rollback();
    }
    let summary = writer.summary.clone();
    drop(writer);

    if summary.added_battle_count > 0 {
        let state = match load_overlay_cache(history_dir, account_id)? {
            Some(cache) => OverlayState::from_history(&cache),
            None => empty_overlay_state(),
        };
        save_overlay_state(history_dir, account_id, &state)?;
    }
    written.map_err(|error| format!("导入中断：{error}"))?;
    Ok(summary)
}

/// 导入时每一遍对每条记录做的事：第一遍只检查，第二遍写入。
trait ImportPass {
    /// 原始 JSON 读成什么：检查时只确认是对象，写入时读成完整的值。
    type Raw: de::DeserializeOwned;

    fn battle(&mut self, item: ImportedRecord<BattleRecord, Self::Raw>) -> Result<(), String>;
    fn coop(&mut self, item: ImportedRecord<CoopRecord, Self::Raw>) -> Result<(), String>;
    fn measurement(&mut self, item: ImportedMeasurement<Self::Raw>) -> Result<(), String>;
}

#[derive(Deserialize)]
struct ImportedRecord<T, R> {
    record: T,
    raw: R,
}

#[derive(Deserialize)]
struct ImportedMeasurement<R> {
    record_id: String,
    payload_hash: String,
    fetched_at_ms: i64,
    raw: R,
}

/// 检查那一遍里的原始 JSON：只确认是一个对象，不在内存里建出来。
struct RawObject;

impl<'de> Deserialize<'de> for RawObject {
    fn deserialize<D: de::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct ObjectVisitor;

        impl<'de> Visitor<'de> for ObjectVisitor {
            type Value = RawObject;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a JSON object")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<RawObject, A::Error> {
                while map
                    .next_entry::<de::IgnoredAny, de::IgnoredAny>()?
                    .is_some()
                {}
                Ok(RawObject)
            }
        }

        deserializer.deserialize_map(ObjectVisitor)
    }
}

#[derive(Default)]
struct ArchiveHeader {
    schema_version: Option<i64>,
    account_id: Option<String>,
    has_battles: bool,
    has_coop: bool,
}

/// 逐条读取导出文件，每读出一条就交给 `pass`。
fn read_archive(archive: &[u8], pass: &mut impl ImportPass) -> Result<ArchiveHeader, String> {
    let reader: Box<dyn Read + '_> = if archive.starts_with(&[0x1f, 0x8b]) {
        Box::new(GzDecoder::new(archive))
    } else {
        Box::new(archive)
    };
    let mut failure = None;
    let mut deserializer = serde_json::Deserializer::from_reader(std::io::BufReader::new(reader));
    let parsed = serde::Deserializer::deserialize_map(
        &mut deserializer,
        ArchiveReader {
            pass,
            failure: &mut failure,
        },
    )
    .and_then(|header| deserializer.end().map(|()| header));
    if let Some(message) = failure {
        return Err(message);
    }
    parsed.map_err(|error| match error.classify() {
        serde_json::error::Category::Io | serde_json::error::Category::Eof => {
            "导入文件不完整或已损坏".to_owned()
        }
        _ => "不是本应用导出的历史文件".to_owned(),
    })
}

struct ArchiveReader<'a, P> {
    pass: &'a mut P,
    failure: &'a mut Option<String>,
}

impl<'de, P: ImportPass> Visitor<'de> for ArchiveReader<'_, P> {
    type Value = ArchiveHeader;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("an exported history archive")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<ArchiveHeader, A::Error> {
        let mut header = ArchiveHeader::default();
        while let Some(key) = map.next_key::<String>()? {
            let kind = match key.as_str() {
                "schema_version" => {
                    header.schema_version = Some(map.next_value()?);
                    continue;
                }
                "account_id" => {
                    header.account_id = Some(map.next_value()?);
                    continue;
                }
                "battles" => {
                    header.has_battles = true;
                    ItemKind::Battle
                }
                "coop" => {
                    header.has_coop = true;
                    ItemKind::Coop
                }
                "x_battle_measurements" => ItemKind::Measurement,
                _ => {
                    map.next_value::<de::IgnoredAny>()?;
                    continue;
                }
            };
            map.next_value_seed(ItemList {
                kind,
                pass: &mut *self.pass,
                failure: &mut *self.failure,
            })?;
        }
        Ok(header)
    }
}

#[derive(Clone, Copy)]
enum ItemKind {
    Battle,
    Coop,
    Measurement,
}

struct ItemList<'a, P> {
    kind: ItemKind,
    pass: &'a mut P,
    failure: &'a mut Option<String>,
}

impl<'de, P: ImportPass> DeserializeSeed<'de> for ItemList<'_, P> {
    type Value = ();

    fn deserialize<D: de::Deserializer<'de>>(self, deserializer: D) -> Result<(), D::Error> {
        deserializer.deserialize_seq(self)
    }
}

impl<'de, P: ImportPass> Visitor<'de> for ItemList<'_, P> {
    type Value = ();

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("a list of archived records")
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<(), A::Error> {
        loop {
            let handled = match self.kind {
                ItemKind::Battle => match seq.next_element()? {
                    Some(item) => self.pass.battle(item),
                    None => return Ok(()),
                },
                ItemKind::Coop => match seq.next_element()? {
                    Some(item) => self.pass.coop(item),
                    None => return Ok(()),
                },
                ItemKind::Measurement => match seq.next_element()? {
                    Some(item) => self.pass.measurement(item),
                    None => return Ok(()),
                },
            };
            if let Err(message) = handled {
                *self.failure = Some(message);
                return Err(de::Error::custom("import stopped"));
            }
        }
    }
}

/// 第一遍：只确认每条记录都读得出来，不写入。
struct ImportCheck;

impl ImportPass for ImportCheck {
    type Raw = RawObject;

    fn battle(&mut self, item: ImportedRecord<BattleRecord, RawObject>) -> Result<(), String> {
        check_imported(&[&item.record.id])
    }

    fn coop(&mut self, item: ImportedRecord<CoopRecord, RawObject>) -> Result<(), String> {
        check_imported(&[&item.record.id])
    }

    fn measurement(&mut self, item: ImportedMeasurement<RawObject>) -> Result<(), String> {
        check_imported(&[&item.record_id, &item.payload_hash])
    }
}

fn check_imported(identifiers: &[&str]) -> Result<(), String> {
    if identifiers.iter().any(|identifier| identifier.is_empty()) {
        return Err("导入文件里有不完整的记录".to_owned());
    }
    Ok(())
}

/// 第二遍：写入本地没有的记录，和同步时一样建卡片索引和面板数据。
struct ImportWriter<'a> {
    connection: Connection,
    account_id: &'a str,
    known_battles: HashSet<String>,
    known_coop: HashSet<String>,
    in_batch: usize,
    /// 还没提交的这一批。
    batch: ImportSummary,
    /// 已经提交的部分。
    summary: ImportSummary,
}

impl ImportWriter<'_> {
    fn begin(&mut self) -> Result<(), String> {
        if self.in_batch == 0 {
            self.connection
                .execute_batch("BEGIN IMMEDIATE")
                .map_err(|error| format!("开启导入事务失败：{error}"))?;
        }
        Ok(())
    }

    fn wrote(&mut self) -> Result<(), String> {
        self.in_batch += 1;
        if self.in_batch >= IMPORT_BATCH_SIZE {
            self.commit()?;
        }
        Ok(())
    }

    fn commit(&mut self) -> Result<(), String> {
        if self.in_batch > 0 {
            self.connection
                .execute_batch("COMMIT")
                .map_err(|error| format!("提交导入事务失败：{error}"))?;
            self.in_batch = 0;
            let batch = std::mem::take(&mut self.batch);
            self.summary.added_battle_count += batch.added_battle_count;
            self.summary.added_coop_count += batch.added_coop_count;
            self.summary.skipped_count += batch.skipped_count;
        }
        Ok(())
    }

    fn rollback(&mut self) {
        if self.in_batch > 0 {
            let _ = self.connection.execute_batch("ROLLBACK");
            self.in_batch = 0;
            self.batch = ImportSummary::default();
        }
    }
}

impl ImportPass for ImportWriter<'_> {
    type Raw = Value;

    fn battle(&mut self, item: ImportedRecord<BattleRecord, Value>) -> Result<(), String> {
        let ImportedRecord { mut record, raw } = item;
        if !self.known_battles.insert(record.id.clone()) {
            self.summary.skipped_count += 1;
            return Ok(());
        }
        refresh_event_power(&mut record, &raw);
        let track_key = archived_battle_power_track_key(&record, &raw);
        self.begin()?;
        let inserted = insert_new_record(
            &self.connection,
            self.account_id,
            "battle",
            &record.id,
            record.played_time.as_deref(),
            &record,
            &raw,
            track_key.as_deref().unwrap_or(NO_OVERLAY_TRACK),
        )?;
        if inserted {
            dashboard_store::upsert_battle(&self.connection, self.account_id, &record, &raw)?;
            self.batch.added_battle_count += 1;
        } else {
            self.batch.skipped_count += 1;
        }
        self.wrote()
    }

    fn coop(&mut self, item: ImportedRecord<CoopRecord, Value>) -> Result<(), String> {
        let ImportedRecord { record, raw } = item;
        if !self.known_coop.insert(record.id.clone()) {
            self.summary.skipped_count += 1;
            return Ok(());
        }
        self.begin()?;
        let inserted = insert_new_record(
            &self.connection,
            self.account_id,
            "coop",
            &record.id,
            record.played_time.as_deref(),
            &record,
            &raw,
            NO_OVERLAY_TRACK,
        )?;
        if inserted {
            dashboard_store::upsert_coop(&self.connection, self.account_id, &record, &raw)?;
            self.batch.added_coop_count += 1;
        } else {
            self.batch.skipped_count += 1;
        }
        self.wrote()
    }

    fn measurement(&mut self, item: ImportedMeasurement<Value>) -> Result<(), String> {
        let raw_json = encode_raw_json(&item.raw)?;
        self.begin()?;
        self.connection
            .execute(
                "INSERT OR IGNORE INTO x_battle_measurement_snapshots(
                     account_id, record_id, payload_hash, fetched_at_ms, raw_json, raw_encoding
                 ) VALUES (?1, ?2, ?3, ?4, ?5, 'gzip')",
                params![
                    self.account_id,
                    item.record_id,
                    item.payload_hash,
                    item.fetched_at_ms,
                    raw_json
                ],
            )
            .map_err(|error| format!("导入 X 赛结算原始数据失败：{error}"))?;
        self.wrote()
    }
}

/// 写入一条本地还没有的记录；已有同一 ID 的记录时什么也不改，返回 false。
#[allow(clippy::too_many_arguments)]
fn insert_new_record<T: Serialize>(
    connection: &Connection,
    account_id: &str,
    record_type: &str,
    record_id: &str,
    played_time: Option<&str>,
    record: &T,
    raw: &Value,
    overlay_track_key: &str,
) -> Result<bool, String> {
    let record_json =
        serde_json::to_string(record).map_err(|error| format!("序列化历史记录失败：{error}"))?;
    let raw_json = encode_raw_json(raw)?;
    let inserted = connection
        .execute(
            "INSERT INTO history_records(
                 account_id, record_type, record_id, played_time,
                 record_json, raw_json, raw_encoding, overlay_track_key
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'gzip', ?7)
             ON CONFLICT(account_id, record_type, record_id) DO NOTHING",
            params![
                account_id,
                record_type,
                record_id,
                played_time,
                record_json,
                raw_json,
                overlay_track_key
            ],
        )
        .map_err(|error| format!("导入历史记录失败：{error}"))?;
    Ok(inserted > 0)
}

pub(crate) fn delete_account(history_dir: &Path, account_id: &str) -> Result<(), String> {
    if account_id.is_empty()
        || !account_id
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_'))
    {
        return Err("账号 ID 格式无效，已停止删除以保护其他数据".to_owned());
    }
    let mut connection = open_database(history_dir)?;
    let transaction = connection
        .transaction()
        .map_err(|error| format!("开启账号历史删除事务失败：{error}"))?;
    transaction
        .execute(
            "DELETE FROM history_records WHERE account_id = ?1",
            [account_id],
        )
        .map_err(|error| format!("删除账号历史记录失败：{error}"))?;
    transaction
        .execute(
            "DELETE FROM history_meta WHERE account_id = ?1",
            [account_id],
        )
        .map_err(|error| format!("删除账号历史状态失败：{error}"))?;
    transaction
        .execute(
            "DELETE FROM overlay_snapshots WHERE account_id = ?1",
            [account_id],
        )
        .map_err(|error| format!("删除账号卡片缓存失败：{error}"))?;
    delete_extension_account_data(&transaction, account_id)?;
    transaction
        .execute(
            "DELETE FROM x_battle_measurement_snapshots WHERE account_id = ?1",
            [account_id],
        )
        .map_err(|error| format!("删除账号 X 赛测定数据失败：{error}"))?;
    transaction
        .commit()
        .map_err(|error| format!("提交账号历史删除事务失败：{error}"))?;

    remove_file_if_present(&legacy_path(history_dir, account_id), "旧历史文件")?;
    // 导出文件是用户自己的备份，删除账号数据时保留。
    remove_matching_files(
        &history_dir.join(LEGACY_DIRECTORY),
        &format!("{account_id}-"),
        "旧历史归档",
    )
}

fn delete_extension_account_data(
    transaction: &rusqlite::Transaction<'_>,
    account_id: &str,
) -> Result<(), String> {
    let table_names = {
        let mut statement = transaction
            .prepare(
                "SELECT name FROM sqlite_master
                 WHERE type = 'table' AND name GLOB 'extension_[A-Za-z0-9_]*'",
            )
            .map_err(|error| format!("查找扩展账号数据表失败：{error}"))?;
        let rows = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| format!("读取扩展账号数据表失败：{error}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("读取扩展账号数据表失败：{error}"))?
    };

    for table_name in table_names {
        if !table_name.strip_prefix("extension_").is_some_and(|suffix| {
            !suffix.is_empty()
                && suffix
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        }) {
            continue;
        }
        let columns = {
            let mut statement = transaction
                .prepare(&format!("PRAGMA table_info(\"{table_name}\")"))
                .map_err(|error| format!("检查扩展账号数据表失败：{error}"))?;
            let rows = statement
                .query_map([], |row| row.get::<_, String>(1))
                .map_err(|error| format!("读取扩展账号数据列失败：{error}"))?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|error| format!("读取扩展账号数据列失败：{error}"))?
        };
        if columns.iter().any(|column| column == "account_id") {
            transaction
                .execute(
                    &format!("DELETE FROM \"{table_name}\" WHERE account_id = ?1"),
                    [account_id],
                )
                .map_err(|error| format!("删除扩展账号数据失败：{error}"))?;
        }
    }
    Ok(())
}

fn remove_file_if_present(path: &Path, description: &str) -> Result<(), String> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("删除{description}失败：{error}")),
    }
}

fn remove_matching_files(directory: &Path, prefix: &str, description: &str) -> Result<(), String> {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("读取{description}目录失败：{error}")),
    };

    for entry in entries {
        let entry = entry.map_err(|error| format!("读取{description}失败：{error}"))?;
        let file_name = entry.file_name();
        let is_file = entry
            .file_type()
            .map_err(|error| format!("读取{description}类型失败：{error}"))?
            .is_file();
        if file_name.to_string_lossy().starts_with(prefix) && is_file {
            remove_file_if_present(&entry.path(), description)?;
        }
    }
    Ok(())
}

fn encode_raw_json(raw: &Value) -> Result<Vec<u8>, String> {
    let bytes = serde_json::to_vec(raw).map_err(|error| format!("序列化原始记录失败：{error}"))?;
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(&bytes)
        .map_err(|error| format!("压缩原始记录失败：{error}"))?;
    encoder
        .finish()
        .map_err(|error| format!("完成原始记录压缩失败：{error}"))
}

fn overlay_cache_raw(track_key: &str) -> Value {
    if track_key == NO_OVERLAY_TRACK {
        return json!({});
    }
    let mut raw = serde_json::Map::new();
    raw.insert(
        CACHED_POWER_TRACK_KEY.to_owned(),
        Value::String(track_key.to_owned()),
    );
    Value::Object(raw)
}

/// 读取一条记录归档的原始 JSON（只读），对战记录页的详情用。
pub(crate) fn load_record_raw(
    history_dir: &Path,
    account_id: &str,
    record_type: &str,
    record_id: &str,
) -> Result<Option<Value>, String> {
    if !matches!(record_type, "battle" | "coop") {
        return Err(format!("不支持的记录类型：{record_type}"));
    }
    let connection = open_database(history_dir)?;
    connection
        .query_row(
            "SELECT raw_json, raw_encoding FROM history_records
             WHERE account_id = ?1 AND record_type = ?2 AND record_id = ?3",
            params![account_id, record_type, record_id],
            |row| Ok((row.get::<_, Vec<u8>>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|error| format!("读取记录详情失败：{error}"))?
        .map(|(bytes, encoding)| decode_raw_json(&bytes, &encoding))
        .transpose()
}

pub(crate) fn decode_raw_json(bytes: &[u8], encoding: &str) -> Result<Value, String> {
    let decoded = if encoding == "gzip" {
        let mut decoder = GzDecoder::new(bytes);
        let mut decoded = Vec::new();
        decoder
            .read_to_end(&mut decoded)
            .map_err(|error| format!("解压原始记录失败：{error}"))?;
        decoded
    } else {
        bytes.to_vec()
    };
    serde_json::from_slice(&decoded).map_err(|error| format!("解析原始记录失败：{error}"))
}

/// 其他模块的测试共用的样例数据。
#[cfg(test)]
pub(crate) mod tests_support {
    use splat3_widget_core::history::HistoryCache;

    /// 一场对战加一场打工。
    pub(crate) fn sample_cache_with_coop() -> HistoryCache {
        let mut cache = super::tests::sample_cache();
        cache.coop.push(super::tests::sample_coop(
            "coop-1",
            "2026-09-22T01:00:00Z".to_owned(),
        ));
        cache
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    pub(crate) fn sample_cache() -> HistoryCache {
        HistoryCache {
            schema_version: 1,
            fetched_at_ms: 123,
            battles: vec![ArchivedBattle {
                record: BattleRecord {
                    id: "battle-1".to_owned(),
                    played_time: Some("2026-09-20T00:00:00Z".to_owned()),
                    mode: "X_MATCH".to_owned(),
                    bankara_mode: None,
                    rule: None,
                    stage: None,
                    judgement: Some("WIN".to_owned()),
                    player_name: None,
                    weapon: None,
                    paint: None,
                    power: Some(2100.0),
                    power_kind: Some("x".to_owned()),
                    kills: Some(8),
                    assists: Some(2),
                    deaths: Some(3),
                    specials: None,
                },
                raw: serde_json::json!({"id":"battle-1","large":"raw"}),
            }],
            coop: vec![],
        }
    }

    pub(crate) fn sample_coop(id: &str, played_time: String) -> ArchivedCoop {
        ArchivedCoop {
            record: CoopRecord {
                id: id.to_owned(),
                played_time: Some(played_time),
                rule: Some("REGULAR".to_owned()),
                stage: None,
                result_wave: None,
                clear_waves: None,
                job_point: None,
                after_grade: None,
                after_grade_point: None,
                danger_rate: None,
                golden_eggs: None,
                power_eggs: None,
                rescued: None,
                rescued_by_others: None,
                rotation_start_time: None,
                rotation_end_time: None,
            },
            raw: json!({ "id": id, "raw": "complete-coop-response" }),
        }
    }

    #[test]
    fn stores_deduplicated_compressed_records_and_loads_them() {
        let directory = tempdir().unwrap();
        let cache = sample_cache();
        save_cache(directory.path(), "account", &cache).unwrap();
        let mut updated = cache.clone();
        updated.battles[0].raw = serde_json::json!({"id":"battle-1","new":true});
        save_cache(directory.path(), "account", &updated).unwrap();
        let loaded = load_cache(directory.path(), "account").unwrap().unwrap();
        assert_eq!(loaded.battles[0].raw, cache.battles[0].raw);
        let connection = open_database(directory.path()).unwrap();
        let count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM history_records WHERE account_id = 'account'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn database_schema_does_not_create_legacy_random_weapon_storage() {
        let directory = tempdir().unwrap();
        let connection = open_database(directory.path()).unwrap();
        let table_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type = 'table' AND name = 'private_weapon_sets'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(table_count, 0);
    }

    #[test]
    fn upgrades_existing_history_database_without_losing_archived_json() {
        let directory = tempdir().unwrap();
        let old_database = Connection::open(database_path(directory.path())).unwrap();
        old_database
            .execute_batch(
                "CREATE TABLE history_records (
                     account_id TEXT NOT NULL,
                     record_type TEXT NOT NULL,
                     record_id TEXT NOT NULL,
                     played_time TEXT,
                     record_json TEXT NOT NULL,
                     raw_json BLOB NOT NULL,
                     raw_encoding TEXT NOT NULL DEFAULT 'gzip',
                     PRIMARY KEY (account_id, record_type, record_id)
                 );
                 CREATE TABLE history_meta (
                     account_id TEXT PRIMARY KEY,
                     schema_version INTEGER NOT NULL,
                     fetched_at_ms INTEGER NOT NULL
                 );",
            )
            .unwrap();
        let archived = sample_cache().battles.remove(0);
        let record_json = serde_json::to_string(&archived.record).unwrap();
        let raw_json = encode_raw_json(&archived.raw).unwrap();
        old_database
            .execute(
                "INSERT INTO history_records(
                     account_id, record_type, record_id, played_time, record_json, raw_json
                 ) VALUES ('account', 'battle', ?1, ?2, ?3, ?4)",
                params![
                    archived.record.id,
                    archived.record.played_time,
                    record_json,
                    raw_json
                ],
            )
            .unwrap();
        drop(old_database);

        let upgraded = open_database(directory.path()).unwrap();
        let upgraded_columns: i64 = upgraded
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('history_records')
                 WHERE name = 'overlay_track_key'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let upgraded_meta: i64 = upgraded
            .query_row(
                "SELECT COUNT(*) FROM pragma_table_info('history_meta')
                 WHERE name = 'sync_complete'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(upgraded_columns, 1);
        assert_eq!(upgraded_meta, 1);
        let (raw_bytes, encoding): (Vec<u8>, String) = upgraded
            .query_row(
                "SELECT raw_json, raw_encoding FROM history_records
                 WHERE account_id = 'account' AND record_id = 'battle-1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            decode_raw_json(&raw_bytes, &encoding).unwrap(),
            archived.raw
        );
    }

    #[test]
    fn overlay_state_is_cached_and_refreshed_without_reading_raw_history() {
        let directory = tempdir().unwrap();
        save_cache(directory.path(), "account", &sample_cache()).unwrap();
        let connection = open_database(directory.path()).unwrap();
        connection
            .execute(
                "DELETE FROM overlay_snapshots WHERE account_id = 'account'",
                [],
            )
            .unwrap();
        drop(connection);

        let first = load_overlay_state(directory.path(), "account").unwrap();
        assert_eq!(
            first.selected.as_ref().and_then(|battle| battle.power),
            Some(2100.0)
        );

        let connection = open_database(directory.path()).unwrap();
        connection
            .execute(
                "UPDATE overlay_snapshots SET state_json = 'not-json' WHERE account_id = 'account'",
                [],
            )
            .unwrap();
        drop(connection);
        let recovered = load_overlay_state(directory.path(), "account").unwrap();
        assert_eq!(
            recovered.selected.as_ref().and_then(|battle| battle.power),
            Some(2100.0)
        );

        let mut updated = recovered;
        updated.selected.as_mut().unwrap().power = Some(2200.0);
        save_overlay_state(directory.path(), "account", &updated).unwrap();

        let connection = open_database(directory.path()).unwrap();
        connection
            .execute(
                "UPDATE history_records SET raw_json = X'00' WHERE account_id = 'account'",
                [],
            )
            .unwrap();
        drop(connection);

        let cached = load_overlay_state(directory.path(), "account").unwrap();
        assert_eq!(
            cached.selected.as_ref().and_then(|battle| battle.power),
            Some(2200.0)
        );
    }

    #[test]
    fn card_data_migration_rebuilds_stale_overlay_snapshots() {
        let directory = tempdir().unwrap();
        let mut latest = sample_cache().battles.remove(0);
        latest.record.id = "open-latest".to_owned();
        latest.record.played_time = Some("2026-09-22T03:00:00Z".to_owned());
        latest.record.mode = "BANKARA".to_owned();
        latest.record.bankara_mode = Some("OPEN".to_owned());
        latest.record.power = Some(2031.8);
        latest.record.power_kind = Some("team".to_owned());
        latest.raw = json!({"myTeam":{"players":[{"id":"latest-a"},{"id":"latest-b"}]}});
        let mut previous = latest.clone();
        previous.record.id = "open-previous".to_owned();
        previous.record.played_time = Some("2026-09-22T02:00:00Z".to_owned());
        previous.record.power = Some(2023.9);
        previous.raw = json!({"myTeam":{"players":[{"id":"previous-a"},{"id":"previous-b"}]}});
        let mut cache = sample_cache();
        cache.battles = vec![latest, previous];
        save_cache(directory.path(), "account", &cache).unwrap();

        let connection = open_database(directory.path()).unwrap();
        connection
            .execute(
                "INSERT INTO history_migrations(account_id, migration_key, version)
                 VALUES (?1, ?2, 1)",
                params!["account", EVENT_POWER_MIGRATION],
            )
            .unwrap();
        drop(connection);

        let old_cache = load_overlay_cache(directory.path(), "account")
            .unwrap()
            .unwrap();
        let mut stale_state = OverlayState::from_history(&old_cache);
        stale_state.selected.as_mut().unwrap().power_delta = Some(0.0);
        save_overlay_state(directory.path(), "account", &stale_state).unwrap();

        let refreshed = load_overlay_state(directory.path(), "account").unwrap();
        let delta = refreshed.selected.unwrap().power_delta.unwrap();
        assert!((delta - 7.9).abs() < 0.0001);
    }

    #[test]
    fn rating_run_migration_reindexes_event_battles_without_a_team() {
        let directory = tempdir().unwrap();
        let mut cache = sample_cache();
        let template = cache.battles.remove(0);
        // SplatNet's shape: the event without a team id, and no power over the first four games.
        cache.battles = (1..=5)
            .map(|index| {
                let mut battle = template.clone();
                battle.record.id = format!("event-{index}");
                battle.record.played_time = Some(format!("2026-09-22T00:0{index}:00Z"));
                battle.record.mode = "EVENT_MATCH".to_owned();
                battle.record.power = (index == 5).then_some(1789.5);
                battle.record.power_kind = Some("event".to_owned());
                battle.raw = json!({
                    "id": battle.record.id,
                    "vsMode": { "mode": "LEAGUE" },
                    "leagueMatch": {
                        "myLeaguePower": battle.record.power,
                        "leagueMatchEvent": { "id": "event-a", "name": "Test Event" }
                    }
                });
                battle
            })
            .collect();
        save_cache(directory.path(), "account", &cache).unwrap();

        // Earlier versions indexed these battles without a track and cached a card without power.
        let connection = open_database(directory.path()).unwrap();
        connection
            .execute(
                "UPDATE history_records SET overlay_track_key = ?2 WHERE account_id = ?1",
                params!["account", NO_OVERLAY_TRACK],
            )
            .unwrap();
        for migration in [EVENT_POWER_MIGRATION, MODE_DELTA_MIGRATION] {
            connection
                .execute(
                    "INSERT INTO history_migrations(account_id, migration_key, version)
                     VALUES (?1, ?2, 1)",
                    params!["account", migration],
                )
                .unwrap();
        }
        drop(connection);
        let old_cache = load_overlay_cache(directory.path(), "account")
            .unwrap()
            .unwrap();
        let stale_state = OverlayState::from_history(&old_cache);
        assert_eq!(stale_state.selected.as_ref().unwrap().power, None);
        save_overlay_state(directory.path(), "account", &stale_state).unwrap();

        let refreshed = load_overlay_state(directory.path(), "account").unwrap();
        let selected = refreshed.selected.unwrap();
        assert_eq!(selected.id, "event-5");
        assert_eq!(selected.power, Some(1789.5));
        assert_eq!(selected.power_delta, Some(0.0));

        let connection = open_database(directory.path()).unwrap();
        let untracked: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM history_records
                 WHERE account_id = ?1 AND overlay_track_key = ?2",
                params!["account", NO_OVERLAY_TRACK],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(untracked, 0);
        let migrated: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM history_migrations
                 WHERE account_id = ?1 AND migration_key = ?2 AND version = 1",
                params!["account", RATING_RUN_MIGRATION],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(migrated, 1);
    }

    #[test]
    fn deletes_account_records_but_keeps_export_files() {
        let directory = tempdir().unwrap();
        save_cache(directory.path(), "account", &sample_cache()).unwrap();
        HistoryRecordWriter::open(directory.path())
            .unwrap()
            .save(
                "account",
                456,
                HistoryFetchedRecord::XBattleSettlement {
                    record_id: "battle-1".to_owned(),
                    power_after: 2110.0,
                    payload_hash: "a".repeat(64),
                    raw_group: serde_json::json!({ "xMatchMeasurement": { "state": "COMPLETED" } }),
                },
            )
            .unwrap();
        let connection = open_database(directory.path()).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE extension_test_data (
                     account_id TEXT PRIMARY KEY,
                     set_json TEXT NOT NULL
                 );
                 INSERT INTO extension_test_data(account_id, set_json)
                 VALUES ('account', '{}');",
            )
            .unwrap();
        drop(connection);
        let export_path = export(directory.path(), "account").unwrap();
        load_overlay_state(directory.path(), "account").unwrap();
        fs::write(
            legacy_path(directory.path(), "account"),
            serde_json::to_vec(&sample_cache()).unwrap(),
        )
        .unwrap();
        let legacy_archive = directory
            .path()
            .join(LEGACY_DIRECTORY)
            .join("account-old.json");
        fs::create_dir_all(legacy_archive.parent().unwrap()).unwrap();
        fs::write(&legacy_archive, b"legacy").unwrap();

        delete_account(directory.path(), "account").unwrap();

        assert!(load_cache(directory.path(), "account").unwrap().is_none());
        let connection = open_database(directory.path()).unwrap();
        let snapshot_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM overlay_snapshots WHERE account_id = 'account'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(snapshot_count, 0);
        let measurement_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM x_battle_measurement_snapshots WHERE account_id = 'account'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(measurement_count, 0);
        let extension_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM extension_test_data WHERE account_id = 'account'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(extension_count, 0);
        assert!(export_path.exists());
        assert!(!legacy_path(directory.path(), "account").exists());
        assert!(!legacy_archive.exists());
    }

    #[test]
    fn refuses_unsafe_account_ids_before_deleting_any_files() {
        let directory = tempdir().unwrap();
        save_cache(directory.path(), "account", &sample_cache()).unwrap();

        assert!(delete_account(directory.path(), "../account").is_err());
        assert!(load_cache(directory.path(), "account").unwrap().is_some());
    }

    #[test]
    fn checkpoint_state_survives_an_interrupted_sync() {
        let directory = tempdir().unwrap();
        let cache = sample_cache();
        assert!(needs_full_sync(directory.path(), "account").unwrap());

        mark_sync_started(directory.path(), "account", 456).unwrap();
        assert!(needs_full_sync(directory.path(), "account").unwrap());

        HistoryRecordWriter::open(directory.path())
            .unwrap()
            .save(
                "account",
                456,
                HistoryFetchedRecord::Battle(cache.battles[0].clone()),
            )
            .unwrap();
        let loaded = load_cache(directory.path(), "account").unwrap().unwrap();
        assert_eq!(loaded.battles.len(), 1);
        assert_eq!(loaded.fetched_at_ms, 456);
        assert!(needs_full_sync(directory.path(), "account").unwrap());

        mark_sync_complete(directory.path(), "account", 456).unwrap();
        assert!(!needs_full_sync(directory.path(), "account").unwrap());
        check_integrity(directory.path()).unwrap();
    }

    #[test]
    fn x_battle_settlement_updates_score_and_archives_unique_compressed_group() {
        let directory = tempdir().unwrap();
        let mut cache = sample_cache();
        cache.battles[0].record.power = Some(2019.7911754988568);
        save_cache(directory.path(), "account", &cache).unwrap();
        let group = serde_json::json!({
            "historyDetails": { "nodes": [{ "id": "battle-1" }] },
            "xMatchMeasurement": {
                "state": "COMPLETED",
                "winCount": 3,
                "loseCount": 0,
                "xPowerAfter": 2057.026
            }
        });
        let mut writer = HistoryRecordWriter::open(directory.path()).unwrap();
        let settlement = || HistoryFetchedRecord::XBattleSettlement {
            record_id: "battle-1".to_owned(),
            power_after: 2057.026,
            payload_hash: "b".repeat(64),
            raw_group: group.clone(),
        };

        assert!(writer.save("account", 789, settlement()).unwrap());
        assert!(!writer.save("account", 790, settlement()).unwrap());
        drop(writer);

        let updated = load_cache(directory.path(), "account").unwrap().unwrap();
        assert_eq!(updated.battles[0].record.power, Some(2057.026));
        save_overlay_state(
            directory.path(),
            "account",
            &OverlayState::from_history(&updated),
        )
        .unwrap();
        let state = load_overlay_state(directory.path(), "account").unwrap();
        assert_eq!(
            state.selected.as_ref().and_then(|battle| battle.power),
            Some(2057.026)
        );

        let connection = open_database(directory.path()).unwrap();
        let (raw, encoding): (Vec<u8>, String) = connection
            .query_row(
                "SELECT raw_json, raw_encoding FROM x_battle_measurement_snapshots
                 WHERE account_id = 'account' AND record_id = 'battle-1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(encoding, "gzip");
        assert_eq!(decode_raw_json(&raw, &encoding).unwrap(), group);
        let snapshot_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM x_battle_measurement_snapshots WHERE account_id = 'account'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(snapshot_count, 1);
    }

    #[test]
    fn record_writer_reuses_database_and_id_lookup_does_not_decode_raw_json() {
        let directory = tempdir().unwrap();
        let cache = sample_cache();
        let mut writer = HistoryRecordWriter::open(directory.path()).unwrap();
        writer
            .save(
                "account",
                456,
                HistoryFetchedRecord::Battle(cache.battles[0].clone()),
            )
            .unwrap();
        let mut second = cache.battles[0].clone();
        second.record.id = "battle-2".to_owned();
        second.raw["id"] = Value::String("battle-2".to_owned());
        writer
            .save("account", 456, HistoryFetchedRecord::Battle(second))
            .unwrap();
        drop(writer);

        let connection = open_database(directory.path()).unwrap();
        connection
            .execute(
                "UPDATE history_records SET raw_json = X'00' WHERE account_id = ?1",
                ["account"],
            )
            .unwrap();
        drop(connection);

        let (battles, coop) = load_known_record_ids(directory.path(), "account", None).unwrap();
        assert_eq!(battles.len(), 2);
        assert!(battles.contains("battle-1"));
        assert!(battles.contains("battle-2"));
        assert!(coop.is_empty());
    }

    #[test]
    fn incremental_id_lookup_limits_each_record_type_independently() {
        let directory = tempdir().unwrap();
        let mut cache = sample_cache();
        cache.battles.clear();
        cache.coop.clear();
        for index in 0..55 {
            let mut battle = sample_cache().battles.remove(0);
            battle.record.id = format!("battle-{index:02}");
            battle.record.played_time = Some(format!("2026-09-22T00:{index:02}:00Z"));
            battle.raw["id"] = Value::String(battle.record.id.clone());
            cache.battles.push(battle);
            cache.coop.push(sample_coop(
                &format!("coop-{index:02}"),
                format!("2026-09-22T00:{index:02}:00Z"),
            ));
        }
        save_cache(directory.path(), "account", &cache).unwrap();

        let (all_battles, all_coop) =
            load_known_record_ids(directory.path(), "account", None).unwrap();
        assert_eq!(all_battles.len(), 55);
        assert_eq!(all_coop.len(), 55);

        let (recent_battles, recent_coop) =
            load_known_record_ids(directory.path(), "account", Some(50)).unwrap();
        assert_eq!(recent_battles.len(), 50);
        assert_eq!(recent_coop.len(), 50);
        assert!(!recent_battles.contains("battle-04"));
        assert!(recent_battles.contains("battle-54"));
        assert!(!recent_coop.contains("coop-04"));
        assert!(recent_coop.contains("coop-54"));
    }

    #[test]
    fn overlay_track_index_backfills_legacy_rows_then_avoids_raw_json() {
        let directory = tempdir().unwrap();
        let mut cache = sample_cache();
        cache.battles[0].record.rule = Some("CLAM".to_owned());
        cache.battles[0].record.power = Some(2020.0);
        cache.battles[0].record.played_time = Some("2026-09-22T03:00:00Z".to_owned());
        let mut previous = cache.battles[0].clone();
        previous.record.id = "battle-previous".to_owned();
        previous.record.power = Some(2000.0);
        previous.record.played_time = Some("2026-09-22T02:00:00Z".to_owned());
        cache.battles.push(previous);
        save_cache(directory.path(), "account", &cache).unwrap();

        let connection = open_database(directory.path()).unwrap();
        connection
            .execute(
                "UPDATE history_records SET overlay_track_key = NULL WHERE account_id = ?1",
                ["account"],
            )
            .unwrap();
        drop(connection);

        let legacy_cache = load_overlay_cache(directory.path(), "account")
            .unwrap()
            .unwrap();
        assert_eq!(
            OverlayState::from_history(&legacy_cache)
                .selected
                .unwrap()
                .power_delta,
            Some(20.0)
        );

        let connection = open_database(directory.path()).unwrap();
        let indexed_rows: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM history_records
                 WHERE account_id = ?1 AND overlay_track_key IS NOT NULL",
                ["account"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(indexed_rows, 2);
        connection
            .execute(
                "UPDATE history_records SET raw_json = X'00' WHERE account_id = ?1",
                ["account"],
            )
            .unwrap();
        drop(connection);

        let indexed_cache = load_overlay_cache(directory.path(), "account")
            .unwrap()
            .unwrap();
        assert_eq!(
            OverlayState::from_history(&indexed_cache)
                .selected
                .unwrap()
                .power_delta,
            Some(20.0)
        );
    }

    #[test]
    fn overlay_load_reparses_event_power_without_rewriting_archived_raw_json() {
        let directory = tempdir().unwrap();
        let mut cache = sample_cache();
        let mut event = cache.battles.remove(0);
        event.record.mode = "EVENT_MATCH".to_owned();
        event.record.power = None;
        event.record.power_kind = Some("event".to_owned());
        event.raw = json!({
            "id": "battle-1",
            "vsMode": { "mode": "LEAGUE" },
            "leagueMatch": {
                "teamId": "team-a",
                "myLeaguePower": 1789.5570782676411,
                "leagueMatchEvent": { "id": "event-a" }
            }
        });
        let mut archived_raw = event.raw.clone();
        cache.battles = (1..=5)
            .map(|index| {
                let mut battle = event.clone();
                battle.record.id = format!("battle-{index}");
                battle.record.played_time = Some(format!("2026-09-22T00:0{index}:00Z"));
                battle.raw["id"] = Value::String(battle.record.id.clone());
                if index == 5 {
                    archived_raw = battle.raw.clone();
                }
                battle
            })
            .collect();
        save_cache(directory.path(), "account", &cache).unwrap();

        let state = load_overlay_state(directory.path(), "account").unwrap();
        let reparsed = load_cache(directory.path(), "account").unwrap().unwrap();
        assert_eq!(reparsed.battles[0].record.id, "battle-5");
        assert_eq!(reparsed.battles[0].record.power, Some(1789.5570782676411));
        assert_eq!(
            state.selected.as_ref().and_then(|battle| battle.power),
            Some(1789.5570782676411)
        );

        assert_eq!(reparsed.battles[0].raw, archived_raw);
    }

    #[test]
    fn salmon_run_overlay_uses_native_scale_counts_and_latest_match_stats() {
        let directory = tempdir().unwrap();
        let mut cache = sample_cache();
        let mut latest = sample_coop("coop-latest", "2026-09-22T02:00:00Z".to_owned());
        latest.record.stage = Some("Gone Fission Hydroplant".to_owned());
        latest.record.clear_waves = Some(3);
        latest.record.after_grade = Some("Eggsecutive VP".to_owned());
        latest.record.after_grade_point = Some(440.0);
        latest.record.danger_rate = Some(2.49);
        latest.raw = json!({
            "scale": { "gold": 1, "silver": 2, "bronze": 3 },
            "myResult": {
                "defeatEnemyCount": 29,
                "goldenDeliverCount": 27,
                "deliverCount": 1309,
                "rescuedCount": 1,
                "rescueCount": 1
            }
        });

        let mut earlier_clear = sample_coop("coop-clear", "2026-09-22T01:00:00Z".to_owned());
        earlier_clear.record.stage = Some("Gone Fission Hydroplant".to_owned());
        earlier_clear.record.clear_waves = Some(3);
        earlier_clear.raw = json!({ "scale": { "gold": 2, "silver": 0, "bronze": 5 } });

        let mut earlier_failure = sample_coop("coop-failure", "2026-09-22T00:00:00Z".to_owned());
        earlier_failure.record.stage = Some("Gone Fission Hydroplant".to_owned());
        earlier_failure.record.clear_waves = Some(2);
        earlier_failure.raw = json!({ "scale": { "gold": 0, "silver": 4, "bronze": 7 } });

        let mut rotation_boundary =
            sample_coop("coop-next-stage", "2026-09-21T00:00:00Z".to_owned());
        rotation_boundary.record.stage = Some("Sockeye Station".to_owned());
        rotation_boundary.record.clear_waves = Some(3);

        let mut outside_rotation = sample_coop("coop-old", "2026-09-20T00:00:00Z".to_owned());
        outside_rotation.record.stage = Some("Gone Fission Hydroplant".to_owned());
        outside_rotation.record.clear_waves = Some(3);
        outside_rotation.raw = json!({ "scale": { "gold": 50, "silver": 50, "bronze": 50 } });

        cache.coop = vec![
            latest,
            earlier_clear,
            earlier_failure,
            rotation_boundary,
            outside_rotation,
        ];
        save_cache(directory.path(), "account", &cache).unwrap();

        let state = load_overlay_state(directory.path(), "account").unwrap();
        let salmon = state.salmon_run.unwrap();
        assert_eq!(salmon.gold_scale, Some(3));
        assert_eq!(salmon.silver_scale, Some(6));
        assert_eq!(salmon.bronze_scale, Some(15));
        assert_eq!(salmon.defeat_enemy_count, Some(29));
        assert_eq!(salmon.golden_egg_count, Some(27));
        assert_eq!(salmon.power_egg_count, Some(1309));
        assert_eq!(salmon.rescued_count, Some(1));
        assert_eq!(salmon.rescue_count, Some(1));
        assert_eq!(salmon.clear_count, 2);
        assert_eq!(salmon.failure_count, 1);
        assert_eq!(salmon.grade_point, Some(440));
        assert_eq!(salmon.danger_rate, Some(2.49));
    }

    #[test]
    fn salmon_run_rotation_matches_stage_and_supplied_weapons_across_other_modes() {
        let directory = tempdir().unwrap();
        let mut cache = sample_cache();
        let rotation_weapons = json!([
            { "name": "Splattershot" },
            { "name": "Slosher" },
            { "name": "Splat Roller" },
            { "name": "E-liter 4K" }
        ]);
        let shift = |id: &str, time: &str, rule: &str, weapons: &Value, gold: i64| {
            let mut coop = sample_coop(id, time.to_owned());
            coop.record.rule = Some(rule.to_owned());
            coop.record.stage = Some("Spawning Grounds".to_owned());
            coop.record.clear_waves = Some(3);
            coop.raw = json!({
                "coopStage": { "id": "Q29vcFN0YWdlLTE=" },
                "weapons": weapons,
                "scale": { "gold": gold, "silver": 0, "bronze": 0 }
            });
            coop
        };
        let other_weapons = json!([{ "name": "Random" }, { "name": "Random" }]);
        cache.coop = vec![
            shift(
                "latest",
                "2026-09-22T10:00:00Z",
                "REGULAR",
                &rotation_weapons,
                1,
            ),
            // 大型跑和私人打工夹在中间，不再把这一期截断。
            shift(
                "big-run",
                "2026-09-22T09:00:00Z",
                "BIG_RUN",
                &other_weapons,
                100,
            ),
            shift(
                "same-1",
                "2026-09-22T08:00:00Z",
                "REGULAR",
                &rotation_weapons,
                2,
            ),
            shift(
                "private",
                "2026-09-22T07:00:00Z",
                "REGULAR",
                &other_weapons,
                100,
            ),
            shift(
                "same-2",
                "2026-09-21T20:00:00Z",
                "REGULAR",
                &rotation_weapons,
                4,
            ),
            // 上一期恰好也是同一张场地，但发放的武器不同。
            shift(
                "previous",
                "2026-09-21T10:00:00Z",
                "REGULAR",
                &other_weapons,
                100,
            ),
        ];
        save_cache(directory.path(), "account", &cache).unwrap();

        let salmon = load_overlay_state(directory.path(), "account")
            .unwrap()
            .salmon_run
            .unwrap();
        assert_eq!(salmon.gold_scale, Some(7));
        assert_eq!(salmon.clear_count, 3);
        assert_eq!(salmon.failure_count, 0);
    }

    #[test]
    fn loads_one_record_raw_json_for_the_matching_account_only() {
        let directory = tempdir().unwrap();
        let mut cache = sample_cache();
        let coop = sample_coop("coop-1", "2026-09-22T01:00:00Z".to_owned());
        let coop_raw = coop.raw.clone();
        cache.coop.push(coop);
        save_cache(directory.path(), "account", &cache).unwrap();

        let battle_id = cache.battles[0].record.id.clone();
        assert_eq!(
            load_record_raw(directory.path(), "account", "battle", &battle_id).unwrap(),
            Some(cache.battles[0].raw.clone())
        );
        assert_eq!(
            load_record_raw(directory.path(), "account", "coop", "coop-1").unwrap(),
            Some(coop_raw)
        );
        assert_eq!(
            load_record_raw(directory.path(), "other", "coop", "coop-1").unwrap(),
            None
        );
        assert_eq!(
            load_record_raw(directory.path(), "account", "battle", "coop-1").unwrap(),
            None
        );
        assert!(load_record_raw(directory.path(), "account", "meta", "coop-1").is_err());
    }

    #[test]
    fn coop_rotation_backfill_only_fills_missing_windows() {
        let directory = tempdir().unwrap();
        let mut writer = HistoryRecordWriter::open(directory.path()).unwrap();
        let coop = sample_coop("coop-1", "2026-09-22T01:00:00Z".to_owned());
        let raw = coop.raw.clone();
        writer
            .save("account", 1, HistoryFetchedRecord::Coop(coop))
            .unwrap();
        let rotation = |start: &str, end: &str| HistoryFetchedRecord::CoopRotation {
            start_time: start.to_owned(),
            end_time: end.to_owned(),
            record_ids: vec!["coop-1".to_owned(), "not-archived".to_owned()],
        };
        writer
            .save(
                "account",
                2,
                rotation("2026-09-21T16:00:00Z", "2026-09-23T08:00:00Z"),
            )
            .unwrap();
        // 已经有起止时间的不再改写。
        writer
            .save(
                "account",
                3,
                rotation("2026-09-23T08:00:00Z", "2026-09-25T00:00:00Z"),
            )
            .unwrap();
        drop(writer);

        let loaded = load_cache(directory.path(), "account").unwrap().unwrap();
        let record = &loaded.coop[0].record;
        assert_eq!(
            record.rotation_start_time.as_deref(),
            Some("2026-09-21T16:00:00Z")
        );
        assert_eq!(
            record.rotation_end_time.as_deref(),
            Some("2026-09-23T08:00:00Z")
        );
        assert_eq!(loaded.coop[0].raw, raw);
    }

    #[test]
    fn salmon_run_rotation_prefers_the_listed_rotation_window() {
        let directory = tempdir().unwrap();
        let mut cache = sample_cache();
        let weapons = json!([{ "name": "Splattershot" }, { "name": "Slosher" }]);
        let shift = |id: &str, time: &str, stage: &str, window: bool, gold: i64| {
            let mut coop = sample_coop(id, time.to_owned());
            coop.record.stage = Some(stage.to_owned());
            coop.record.clear_waves = Some(3);
            if window {
                coop.record.rotation_start_time = Some("2026-09-22T00:00:00Z".to_owned());
                coop.record.rotation_end_time = Some("2026-09-23T16:00:00Z".to_owned());
            }
            coop.raw = json!({
                "weapons": weapons,
                "scale": { "gold": gold, "silver": 0, "bronze": 0 }
            });
            coop
        };
        cache.coop = vec![
            shift(
                "latest",
                "2026-09-22T10:00:00Z",
                "Spawning Grounds",
                true,
                1,
            ),
            shift("same", "2026-09-22T05:00:00Z", "Spawning Grounds", true, 2),
            // 之前同步、没有起止时间的记录：在这一期之内，场地和武器一致，算进来。
            shift(
                "older-sync",
                "2026-09-22T02:00:00Z",
                "Spawning Grounds",
                false,
                4,
            ),
            shift(
                "other-setup",
                "2026-09-22T01:00:00Z",
                "Sockeye Station",
                false,
                100,
            ),
            // 早于这一期开始：即使场地和武器相同、在 48 小时内，也不算。
            shift(
                "before",
                "2026-09-21T23:00:00Z",
                "Spawning Grounds",
                false,
                100,
            ),
        ];
        save_cache(directory.path(), "account", &cache).unwrap();

        let salmon = load_overlay_state(directory.path(), "account")
            .unwrap()
            .salmon_run
            .unwrap();
        assert_eq!(salmon.gold_scale, Some(7));
        assert_eq!(salmon.clear_count, 3);
    }

    /// Two battles, one shift and one X settlement, exported; returns the file's bytes.
    fn exported_sample(directory: &Path) -> Vec<u8> {
        let mut cache = sample_cache();
        let mut second = cache.battles[0].clone();
        second.record.id = "battle-2".to_owned();
        second.record.played_time = Some("2026-09-21T00:00:00Z".to_owned());
        second.raw = json!({ "id": "battle-2", "full": [4, 5, 6] });
        cache.battles.push(second);
        cache
            .coop
            .push(sample_coop("coop-1", "2026-09-22T01:00:00Z".to_owned()));
        save_cache(directory, "account", &cache).unwrap();
        HistoryRecordWriter::open(directory)
            .unwrap()
            .save(
                "account",
                456,
                HistoryFetchedRecord::XBattleSettlement {
                    record_id: "battle-1".to_owned(),
                    power_after: 2110.0,
                    payload_hash: "c".repeat(64),
                    raw_group: json!({ "xMatchMeasurement": { "state": "COMPLETED" } }),
                },
            )
            .unwrap();
        fs::read(export(directory, "account").unwrap()).unwrap()
    }

    fn count_rows(directory: &Path, sql: &str) -> i64 {
        open_database(directory)
            .unwrap()
            .query_row(sql, [], |row| row.get(0))
            .unwrap()
    }

    #[test]
    fn import_restores_an_export_and_keeps_records_already_present() {
        let source = tempdir().unwrap();
        let archive = exported_sample(source.path());

        // The new computer already synced battle-1 and keeps its own copy.
        let target = tempdir().unwrap();
        let mut local = sample_cache();
        local.battles[0].record.kills = Some(1);
        save_cache(target.path(), "account", &local).unwrap();

        let summary = import(target.path(), "account", &archive).unwrap();
        assert_eq!(
            summary,
            ImportSummary {
                added_battle_count: 1,
                added_coop_count: 1,
                skipped_count: 1,
            }
        );
        let original = load_cache(source.path(), "account").unwrap().unwrap();
        let restored = load_cache(target.path(), "account").unwrap().unwrap();
        let battle = |cache: &HistoryCache, id: &str| {
            cache
                .battles
                .iter()
                .find(|battle| battle.record.id == id)
                .cloned()
                .unwrap()
        };
        assert_eq!(battle(&restored, "battle-2"), battle(&original, "battle-2"));
        assert_eq!(restored.coop, original.coop);
        assert_eq!(battle(&restored, "battle-1").record.kills, Some(1));
        assert_eq!(
            count_rows(
                target.path(),
                "SELECT COUNT(*) FROM x_battle_measurement_snapshots"
            ),
            1
        );
        assert_eq!(
            count_rows(
                target.path(),
                "SELECT COUNT(*) FROM extension_dashboard_battles"
            ),
            2
        );
        assert_eq!(
            count_rows(
                target.path(),
                "SELECT COUNT(*) FROM extension_dashboard_coop"
            ),
            1
        );
        // The OBS snapshot now shows the imported, more recent battle.
        let snapshot: String = open_database(target.path())
            .unwrap()
            .query_row("SELECT state_json FROM overlay_snapshots", [], |row| {
                row.get(0)
            })
            .unwrap();
        let snapshot: OverlayState = serde_json::from_str(&snapshot).unwrap();
        assert_eq!(snapshot.selected.unwrap().id, "battle-2");

        // Importing the same file again adds nothing.
        assert_eq!(
            import(target.path(), "account", &archive).unwrap(),
            ImportSummary {
                added_battle_count: 0,
                added_coop_count: 0,
                skipped_count: 3,
            }
        );
    }

    #[test]
    fn import_refuses_other_accounts_and_damaged_files_without_writing() {
        let source = tempdir().unwrap();
        let archive = exported_sample(source.path());
        let target = tempdir().unwrap();

        let refused =
            |account_id: &str, bytes: &[u8]| import(target.path(), account_id, bytes).unwrap_err();
        assert_eq!(
            refused("other", &archive),
            "这份文件是另一个账号导出的，没有导入"
        );
        assert_eq!(
            refused("account", &archive[..archive.len() / 2]),
            "导入文件不完整或已损坏"
        );
        assert_eq!(
            refused("account", b"not an export"),
            "不是本应用导出的历史文件"
        );
        assert_eq!(
            refused("account", br#"{"schema_version":2,"account_id":"account"}"#),
            "不是本应用导出的历史文件"
        );
        assert_eq!(
            refused(
                "account",
                br#"{"schema_version":3,"account_id":"account","battles":[],"coop":[]}"#
            ),
            "这个导出文件的版本不受支持"
        );
        assert_eq!(
            count_rows(target.path(), "SELECT COUNT(*) FROM history_records"),
            0
        );

        // A decompressed export imports too.
        let mut plain = Vec::new();
        GzDecoder::new(archive.as_slice())
            .read_to_end(&mut plain)
            .unwrap();
        assert_eq!(
            import(target.path(), "account", &plain).unwrap(),
            ImportSummary {
                added_battle_count: 2,
                added_coop_count: 1,
                skipped_count: 0,
            }
        );
    }

    #[test]
    fn export_streams_complete_battle_coop_and_settlement_raw_json() {
        let directory = tempdir().unwrap();
        let mut cache = sample_cache();
        let battle_raw = json!({ "id": "battle-1", "full": [1, 2, 3] });
        cache.battles[0].raw = battle_raw.clone();
        let coop_raw = json!({ "id": "coop-1", "full": { "waves": [1, 2, 3] } });
        cache
            .coop
            .push(sample_coop("coop-1", "2026-09-22T01:00:00Z".to_owned()));
        cache.coop[0].raw = coop_raw.clone();
        save_cache(directory.path(), "account", &cache).unwrap();

        let settlement_raw =
            json!({ "xMatchMeasurement": { "state": "COMPLETED", "detail": [4, 5] } });
        HistoryRecordWriter::open(directory.path())
            .unwrap()
            .save(
                "account",
                456,
                HistoryFetchedRecord::XBattleSettlement {
                    record_id: "battle-1".to_owned(),
                    power_after: 2110.0,
                    payload_hash: "c".repeat(64),
                    raw_group: settlement_raw.clone(),
                },
            )
            .unwrap();

        let archive_path = export(directory.path(), "account").unwrap();
        let mut decoder = GzDecoder::new(fs::File::open(archive_path).unwrap());
        let mut bytes = Vec::new();
        decoder.read_to_end(&mut bytes).unwrap();
        let archive: Value = serde_json::from_slice(&bytes).unwrap();

        assert_eq!(archive["battles"][0]["raw"], battle_raw);
        assert_eq!(archive["coop"][0]["raw"], coop_raw);
        assert_eq!(archive["x_battle_measurements"][0]["raw"], settlement_raw);
        assert_eq!(archive["schema_version"], 2);
    }
}
