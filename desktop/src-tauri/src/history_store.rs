use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use flate2::Compression;
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;
use serde_json::{json, Value};
use splat3_widget_core::history::{
    ArchivedBattle, ArchivedCoop, BattleRecord, CoopRecord, HistoryCache, HistoryFetchedRecord,
    HistorySyncSummary,
};
use splat3_widget_core::overlay::{OverlayState, RecentSummary};
use std::collections::HashSet;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const DATABASE_FILE: &str = "history.sqlite3";
const LEGACY_DIRECTORY: &str = "legacy-json";
const EXPORT_DIRECTORY: &str = "exports";
const DATABASE_SCHEMA_VERSION: i64 = 1;

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
                 fetched_at_ms = excluded.fetched_at_ms,
                 sync_complete = 0",
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

pub(crate) fn load_cache(
    history_dir: &Path,
    account_id: &str,
) -> Result<Option<HistoryCache>, String> {
    migrate_legacy_if_needed(history_dir, account_id)?;
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

pub(crate) fn migrate_legacy_if_needed(history_dir: &Path, account_id: &str) -> Result<(), String> {
    let connection = open_database(history_dir)?;
    let record_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM history_records WHERE account_id = ?1",
            [account_id],
            |row| row.get(0),
        )
        .map_err(|error| format!("检查旧历史数据失败：{error}"))?;
    drop(connection);

    if record_count == 0 {
        if let Some(legacy) = load_legacy_cache(history_dir, account_id)? {
            save_cache(history_dir, account_id, &legacy)?;
            archive_legacy_file(history_dir, account_id);
        }
    }
    Ok(())
}

pub(crate) fn load_known_record_ids(
    history_dir: &Path,
    account_id: &str,
) -> Result<(HashSet<String>, HashSet<String>), String> {
    let connection = open_database(history_dir)?;
    let mut statement = connection
        .prepare("SELECT record_type, record_id FROM history_records WHERE account_id = ?1")
        .map_err(|error| format!("准备历史去重查询失败：{error}"))?;
    let rows = statement
        .query_map([account_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| format!("读取历史去重信息失败：{error}"))?;
    let (mut battle_ids, mut coop_ids) = (HashSet::new(), HashSet::new());
    for row in rows {
        let (record_type, record_id) =
            row.map_err(|error| format!("读取历史去重行失败：{error}"))?;
        match record_type.as_str() {
            "battle" => {
                battle_ids.insert(record_id);
            }
            "coop" => {
                coop_ids.insert(record_id);
            }
            other => return Err(format!("历史数据库包含未知记录类型：{other}")),
        }
    }
    Ok((battle_ids, coop_ids))
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
        drop(connection);
        return Ok(load_legacy_cache(history_dir, account_id)?.map(|cache| cache.summary()));
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
    let record_count: i64 = connection
        .query_row(
            "SELECT COUNT(*) FROM history_records
             WHERE account_id = ?1 AND record_type = 'battle'",
            [account_id],
            |row| row.get(0),
        )
        .map_err(|error| format!("读取卡片历史统计失败：{error}"))?;
    if record_count == 0 {
        drop(connection);
        return load_legacy_cache(history_dir, account_id);
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
    let mut statement = connection
        .prepare(
            "SELECT record_json, raw_json, raw_encoding FROM history_records
             WHERE account_id = ?1 AND record_type = 'battle'
             ORDER BY played_time IS NULL, played_time DESC, rowid DESC",
        )
        .map_err(|error| format!("准备卡片历史查询失败：{error}"))?;
    let rows = statement
        .query_map([account_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Vec<u8>>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|error| format!("读取卡片历史失败：{error}"))?;
    let mut battles = Vec::new();
    for row in rows {
        let (record_json, raw_json, raw_encoding) =
            row.map_err(|error| format!("读取卡片历史行失败：{error}"))?;
        let record = serde_json::from_str::<BattleRecord>(&record_json)
            .map_err(|error| format!("解析卡片对战记录失败：{error}"))?;
        let raw = decode_raw_json(&raw_json, &raw_encoding)?;
        battles.push(ArchivedBattle {
            record,
            raw: overlay_identity_raw(&raw),
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
        upsert_record(
            &transaction,
            account_id,
            "battle",
            &battle.record.id,
            battle.record.played_time.as_deref(),
            &battle.record,
            &battle.raw,
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
    ) -> Result<(), String> {
        let transaction = self
            .connection
            .transaction()
            .map_err(|error| format!("开启历史断点事务失败：{error}"))?;
        match record {
            HistoryFetchedRecord::Battle(record) => upsert_record(
                &transaction,
                account_id,
                "battle",
                &record.record.id,
                record.record.played_time.as_deref(),
                &record.record,
                &record.raw,
            )?,
            HistoryFetchedRecord::Coop(record) => upsert_record(
                &transaction,
                account_id,
                "coop",
                &record.record.id,
                record.record.played_time.as_deref(),
                &record.record,
                &record.raw,
            )?,
        }
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
            .map_err(|error| format!("提交历史断点事务失败：{error}"))
    }
}

fn upsert_record<T: Serialize>(
    transaction: &rusqlite::Transaction<'_>,
    account_id: &str,
    record_type: &str,
    record_id: &str,
    played_time: Option<&str>,
    record: &T,
    raw: &Value,
) -> Result<(), String> {
    let record_json =
        serde_json::to_string(record).map_err(|error| format!("序列化历史记录失败：{error}"))?;
    let raw_json = encode_raw_json(raw)?;
    transaction
        .execute(
            "INSERT INTO history_records(
                 account_id, record_type, record_id, played_time,
                 record_json, raw_json, raw_encoding
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'gzip')
             ON CONFLICT(account_id, record_type, record_id) DO UPDATE SET
                 played_time = excluded.played_time,
                 record_json = excluded.record_json",
            params![
                account_id,
                record_type,
                record_id,
                played_time,
                record_json,
                raw_json
            ],
        )
        .map_err(|error| format!("保存历史记录失败：{error}"))?;
    Ok(())
}

pub(crate) fn export(history_dir: &Path, account_id: &str) -> Result<PathBuf, String> {
    let cache = load_cache(history_dir, account_id)?
        .ok_or_else(|| "当前账号还没有可导出的历史数据".to_owned())?;
    let export_dir = history_dir.join(EXPORT_DIRECTORY);
    fs::create_dir_all(&export_dir).map_err(|error| format!("创建导出目录失败：{error}"))?;
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("读取导出时间失败：{error}"))?
        .as_millis();
    let path = export_dir.join(format!("{account_id}-{timestamp}.json.gz"));
    let file = fs::File::create(&path).map_err(|error| format!("创建导出文件失败：{error}"))?;
    let mut encoder = GzEncoder::new(file, Compression::default());
    serde_json::to_writer_pretty(&mut encoder, &cache)
        .map_err(|error| format!("生成导出数据失败：{error}"))?;
    encoder
        .finish()
        .map_err(|error| format!("写入导出文件失败：{error}"))?;
    Ok(path)
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

fn overlay_identity_raw(raw: &Value) -> Value {
    let rule = raw.pointer("/vsRule/rule").cloned().unwrap_or(Value::Null);
    let players = raw
        .pointer("/myTeam/players")
        .and_then(Value::as_array)
        .map(|players| {
            players
                .iter()
                .map(|player| {
                    json!({
                        "isMyself": player.get("isMyself"),
                        "id": player.get("id"),
                        "nameId": player.get("nameId"),
                        "weapon": { "id": player.pointer("/weapon/id") },
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    json!({
        "vsRule": { "rule": rule },
        "myTeam": { "players": players },
    })
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

fn load_legacy_cache(history_dir: &Path, account_id: &str) -> Result<Option<HistoryCache>, String> {
    let path = legacy_path(history_dir, account_id);
    if !path.exists() {
        return Ok(None);
    }
    let contents =
        fs::read_to_string(&path).map_err(|error| format!("读取旧历史缓存失败：{error}"))?;
    serde_json::from_str(&contents)
        .map(Some)
        .map_err(|error| format!("旧历史缓存格式无效：{error}"))
}

fn archive_legacy_file(history_dir: &Path, account_id: &str) {
    let source = legacy_path(history_dir, account_id);
    let directory = history_dir.join(LEGACY_DIRECTORY);
    if fs::create_dir_all(&directory).is_err() {
        return;
    }
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |value| value.as_millis());
    let target = directory.join(format!("{account_id}-{timestamp}.json"));
    let _ = fs::rename(source, target);
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
    fn deletes_account_records_and_export_files() {
        let directory = tempdir().unwrap();
        save_cache(directory.path(), "account", &sample_cache()).unwrap();
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

        let (battles, coop) = load_known_record_ids(directory.path(), "account").unwrap();
        assert_eq!(battles.len(), 2);
        assert!(battles.contains("battle-1"));
        assert!(battles.contains("battle-2"));
        assert!(coop.is_empty());
    }

    #[test]
    fn migrates_legacy_json_and_keeps_a_backup() {
        let directory = tempdir().unwrap();
        fs::create_dir_all(directory.path()).unwrap();
        fs::write(
            legacy_path(directory.path(), "account"),
            serde_json::to_vec(&sample_cache()).unwrap(),
        )
        .unwrap();
        let loaded = load_cache(directory.path(), "account").unwrap().unwrap();
        assert_eq!(loaded.battles.len(), 1);
        assert!(!legacy_path(directory.path(), "account").exists());
        assert!(directory.path().join(LEGACY_DIRECTORY).exists());
    }
}
