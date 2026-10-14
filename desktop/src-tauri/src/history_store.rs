use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::{json, Value};
use splat3_widget_core::history::{
    parse_battle, ArchivedBattle, BattleRecord, HistoryCache, HistoryFetchedRecord,
    HistorySyncSummary,
};
#[cfg(test)]
use splat3_widget_core::history::{ArchivedCoop, CoopRecord};
use splat3_widget_core::overlay::{archived_battle_power_track_key, OverlayState, RecentSummary};
use std::collections::HashSet;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const DATABASE_FILE: &str = "history.sqlite3";
const LEGACY_DIRECTORY: &str = "legacy-json";
const EXPORT_DIRECTORY: &str = "exports";
const DATABASE_SCHEMA_VERSION: i64 = 2;
const NO_OVERLAY_TRACK: &str = "none";
const CACHED_POWER_TRACK_KEY: &str = "__s3w_overlay_power_track_v1";
const EVENT_POWER_MIGRATION: &str = "event-power-fields-v1";
const MODE_DELTA_MIGRATION: &str = "mode-level-deltas-v1";

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

fn open_database(history_dir: &Path) -> Result<Connection, String> {
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

pub(crate) fn load_overlay_state(
    history_dir: &Path,
    account_id: &str,
) -> Result<OverlayState, String> {
    let connection = open_database(history_dir)?;
    let migrated_version = connection
        .query_row(
            "SELECT version FROM history_migrations
             WHERE account_id = ?1 AND migration_key = ?2",
            params![account_id, EVENT_POWER_MIGRATION],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(|error| format!("读取活动赛卡片数据版本失败：{error}"))?;
    if migrated_version != Some(1) {
        drop(connection);
        reparse_event_match_records(history_dir, account_id)?;
        let state = match load_overlay_cache(history_dir, account_id)? {
            Some(cache) => OverlayState::from_history(&cache),
            None => empty_overlay_state(),
        };
        save_overlay_state(history_dir, account_id, &state)?;
        let connection = open_database(history_dir)?;
        connection
            .execute(
                "INSERT INTO history_migrations(account_id, migration_key, version)
                 VALUES (?1, ?2, 1)
                 ON CONFLICT(account_id, migration_key) DO UPDATE SET
                     version = excluded.version",
                params![account_id, EVENT_POWER_MIGRATION],
            )
            .map_err(|error| format!("保存活动赛卡片数据版本失败：{error}"))?;
        connection
            .execute(
                "INSERT INTO history_migrations(account_id, migration_key, version)
                 VALUES (?1, ?2, 1)
                 ON CONFLICT(account_id, migration_key) DO UPDATE SET
                     version = excluded.version",
                params![account_id, MODE_DELTA_MIGRATION],
            )
            .map_err(|error| format!("保存模式差分卡片数据版本失败：{error}"))?;
        return Ok(state);
    }
    let mode_delta_version = connection
        .query_row(
            "SELECT version FROM history_migrations
             WHERE account_id = ?1 AND migration_key = ?2",
            params![account_id, MODE_DELTA_MIGRATION],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(|error| format!("读取模式差分卡片数据版本失败：{error}"))?;
    if mode_delta_version != Some(1) {
        drop(connection);
        let state = match load_overlay_cache(history_dir, account_id)? {
            Some(cache) => OverlayState::from_history(&cache),
            None => empty_overlay_state(),
        };
        save_overlay_state(history_dir, account_id, &state)?;
        let connection = open_database(history_dir)?;
        connection
            .execute(
                "INSERT INTO history_migrations(account_id, migration_key, version)
                 VALUES (?1, ?2, 1)
                 ON CONFLICT(account_id, migration_key) DO UPDATE SET
                     version = excluded.version",
                params![account_id, MODE_DELTA_MIGRATION],
            )
            .map_err(|error| format!("保存模式差分卡片数据版本失败：{error}"))?;
        return Ok(state);
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
            Ok(state) => return Ok(state),
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
            recent_summary: RecentSummary {
                sample_size: 0,
                win: 0,
                lose: 0,
                win_rate: 0.0,
            },
        },
    };
    save_overlay_state(history_dir, account_id, &state)?;
    Ok(state)
}

fn empty_overlay_state() -> OverlayState {
    OverlayState {
        selected: None,
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
        if let Some(parsed) = parse_battle(&raw, &record_id) {
            if parsed.power.is_some() {
                record.power = parsed.power;
                record.power_kind = parsed.power_kind;
            }
        }
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
    remove_matching_files(
        &history_dir.join(LEGACY_DIRECTORY),
        &format!("{account_id}-"),
        "旧历史归档",
    )?;
    remove_matching_files(
        &history_dir.join(EXPORT_DIRECTORY),
        &format!("{account_id}-"),
        "历史导出文件",
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

fn decode_raw_json(bytes: &[u8], encoding: &str) -> Result<Value, String> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn sample_cache() -> HistoryCache {
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

    fn sample_coop(id: &str, played_time: String) -> ArchivedCoop {
        ArchivedCoop {
            record: CoopRecord {
                id: id.to_owned(),
                played_time: Some(played_time),
                rule: None,
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
    fn mode_delta_migration_rebuilds_stale_overlay_snapshots() {
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
    fn deletes_account_records_and_export_files() {
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
        assert!(!export_path.exists());
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
