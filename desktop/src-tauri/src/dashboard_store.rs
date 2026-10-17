//! Dashboard fact tables.
//!
//! The facts are a flat index derived from `history_records`. The archive's
//! `record_json` and compressed raw JSON are only read here, never changed,
//! and the facts can be dropped and rebuilt at any time. The tables use the
//! `extension_` prefix with an `account_id` column, so deleting an account
//! removes them through `history_store::delete_extension_account_data`.

use crate::history_store;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::Serialize;
use serde_json::Value;
use splat3_widget_core::dashboard::{
    battle_fact, coop_fact, BattleFact, CoopFact, CoopWaveFact, DASHBOARD_FACTS_VERSION,
};
use splat3_widget_core::history::{BattleRecord, CoopRecord};
use std::path::Path;
use std::sync::Mutex;

const FACTS_MIGRATION: &str = "dashboard-facts";
const BACKFILL_BATCH_SIZE: i64 = 200;

static FACTS_LOCK: Mutex<()> = Mutex::new(());

pub(crate) fn ensure_schema(connection: &Connection) -> Result<(), String> {
    connection
        .execute_batch(
            "CREATE TABLE IF NOT EXISTS extension_dashboard_battles (
                 account_id TEXT NOT NULL,
                 record_id TEXT NOT NULL,
                 played_at_ms INTEGER,
                 mode TEXT NOT NULL,
                 bankara_mode TEXT,
                 rule_code TEXT,
                 rule_name TEXT,
                 stage_id TEXT,
                 stage_name TEXT,
                 weapon_id TEXT,
                 weapon_name TEXT,
                 weapon_image TEXT,
                 judgement TEXT,
                 knockout TEXT,
                 duration_seconds INTEGER,
                 kills INTEGER,
                 assists INTEGER,
                 deaths INTEGER,
                 specials INTEGER,
                 paint INTEGER,
                 power REAL,
                 power_kind TEXT,
                 PRIMARY KEY (account_id, record_id)
             );
             CREATE INDEX IF NOT EXISTS idx_extension_dashboard_battles_time
                 ON extension_dashboard_battles(account_id, played_at_ms);
             CREATE TABLE IF NOT EXISTS extension_dashboard_coop (
                 account_id TEXT NOT NULL,
                 record_id TEXT NOT NULL,
                 played_at_ms INTEGER,
                 rule TEXT,
                 stage_id TEXT,
                 stage_name TEXT,
                 result_wave INTEGER,
                 cleared INTEGER,
                 waves_json TEXT NOT NULL,
                 boss_appeared INTEGER NOT NULL,
                 boss_defeated INTEGER,
                 boss_id TEXT,
                 boss_name TEXT,
                 danger_rate REAL,
                 grade_name TEXT,
                 grade_point INTEGER,
                 golden_eggs INTEGER,
                 power_eggs INTEGER,
                 rescue_count INTEGER,
                 rescued_count INTEGER,
                 defeat_enemy_count INTEGER,
                 scale_gold INTEGER,
                 scale_silver INTEGER,
                 scale_bronze INTEGER,
                 PRIMARY KEY (account_id, record_id)
             );
             CREATE INDEX IF NOT EXISTS idx_extension_dashboard_coop_time
                 ON extension_dashboard_coop(account_id, played_at_ms);",
        )
        .map_err(|error| format!("初始化面板数据表失败：{error}"))?;
    // 第 2 版事实表增加了武器图片；旧数据库补上这一列，事实行会按版本号重建。
    let has_weapon_image = connection
        .prepare("PRAGMA table_info(extension_dashboard_battles)")
        .and_then(|mut statement| {
            statement
                .query_map([], |row| row.get::<_, String>(1))?
                .collect::<Result<Vec<_>, _>>()
        })
        .map_err(|error| format!("检查面板数据表失败：{error}"))?
        .iter()
        .any(|column| column == "weapon_image");
    if !has_weapon_image {
        connection
            .execute(
                "ALTER TABLE extension_dashboard_battles ADD COLUMN weapon_image TEXT",
                [],
            )
            .map_err(|error| format!("升级面板数据表失败：{error}"))?;
    }
    Ok(())
}

pub(crate) fn upsert_battle(
    connection: &Connection,
    account_id: &str,
    record: &BattleRecord,
    raw: &Value,
) -> Result<(), String> {
    write_battle(connection, account_id, &battle_fact(record, raw))
}

pub(crate) fn upsert_coop(
    connection: &Connection,
    account_id: &str,
    record: &CoopRecord,
    raw: &Value,
) -> Result<(), String> {
    write_coop(connection, account_id, &coop_fact(record, raw))
}

pub(crate) fn update_battle_power(
    connection: &Connection,
    account_id: &str,
    record_id: &str,
    power: f64,
) -> Result<(), String> {
    connection
        .execute(
            "UPDATE extension_dashboard_battles SET power = ?3
             WHERE account_id = ?1 AND record_id = ?2",
            params![account_id, record_id, power],
        )
        .map_err(|error| format!("更新面板 X 赛战力失败：{error}"))?;
    Ok(())
}

fn played_at_ms(played_time: Option<&str>) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(played_time?)
        .ok()
        .map(|time| time.timestamp_millis())
}

fn write_battle(
    connection: &Connection,
    account_id: &str,
    fact: &BattleFact,
) -> Result<(), String> {
    connection
        .execute(
            "INSERT OR REPLACE INTO extension_dashboard_battles(
                 account_id, record_id, played_at_ms, mode, bankara_mode, rule_code, rule_name,
                 stage_id, stage_name, weapon_id, weapon_name, judgement, knockout,
                 duration_seconds, kills, assists, deaths, specials, paint, power, power_kind,
                 weapon_image
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22)",
            params![
                account_id,
                fact.record_id,
                played_at_ms(fact.played_time.as_deref()),
                fact.mode,
                fact.bankara_mode,
                fact.rule_code,
                fact.rule_name,
                fact.stage_id,
                fact.stage_name,
                fact.weapon_id,
                fact.weapon_name,
                fact.judgement,
                fact.knockout,
                fact.duration_seconds,
                fact.kills,
                fact.assists,
                fact.deaths,
                fact.specials,
                fact.paint,
                fact.power,
                fact.power_kind,
                fact.weapon_image,
            ],
        )
        .map_err(|error| format!("保存面板对战数据失败：{error}"))?;
    Ok(())
}

fn write_coop(connection: &Connection, account_id: &str, fact: &CoopFact) -> Result<(), String> {
    let waves_json = serde_json::to_string(&fact.waves)
        .map_err(|error| format!("序列化面板波次数据失败：{error}"))?;
    connection
        .execute(
            "INSERT OR REPLACE INTO extension_dashboard_coop(
                 account_id, record_id, played_at_ms, rule, stage_id, stage_name, result_wave,
                 cleared, waves_json, boss_appeared, boss_defeated, boss_id, boss_name,
                 danger_rate, grade_name, grade_point, golden_eggs, power_eggs, rescue_count,
                 rescued_count, defeat_enemy_count, scale_gold, scale_silver, scale_bronze
             ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24)",
            params![
                account_id,
                fact.record_id,
                played_at_ms(fact.played_time.as_deref()),
                fact.rule,
                fact.stage_id,
                fact.stage_name,
                fact.result_wave,
                fact.cleared,
                waves_json,
                fact.boss_appeared,
                fact.boss_defeated,
                fact.boss_id,
                fact.boss_name,
                fact.danger_rate,
                fact.grade_name,
                fact.grade_point,
                fact.golden_eggs,
                fact.power_eggs,
                fact.rescue_count,
                fact.rescued_count,
                fact.defeat_enemy_count,
                fact.scale_gold,
                fact.scale_silver,
                fact.scale_bronze,
            ],
        )
        .map_err(|error| format!("保存面板打工数据失败：{error}"))?;
    Ok(())
}

/// Makes the account's facts match the archive: rebuilds them when the
/// extraction version changed, then fills in any archived record that has no
/// fact yet. Work is committed in small batches so a running sync is never
/// blocked for long.
pub(crate) fn ensure_facts(history_dir: &Path, account_id: &str) -> Result<(), String> {
    let _guard = FACTS_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let mut connection = history_store::open_database(history_dir)?;
    let version = connection
        .query_row(
            "SELECT version FROM history_migrations
             WHERE account_id = ?1 AND migration_key = ?2",
            params![account_id, FACTS_MIGRATION],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(|error| format!("读取面板数据版本失败：{error}"))?;
    if version != Some(DASHBOARD_FACTS_VERSION) {
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("开启面板数据重建事务失败：{error}"))?;
        for table in ["extension_dashboard_battles", "extension_dashboard_coop"] {
            transaction
                .execute(
                    &format!("DELETE FROM {table} WHERE account_id = ?1"),
                    [account_id],
                )
                .map_err(|error| format!("清理旧版面板数据失败：{error}"))?;
        }
        transaction
            .commit()
            .map_err(|error| format!("提交面板数据重建事务失败：{error}"))?;
    }

    backfill(&mut connection, account_id, "battle")?;
    backfill(&mut connection, account_id, "coop")?;

    if version != Some(DASHBOARD_FACTS_VERSION) {
        connection
            .execute(
                "INSERT INTO history_migrations(account_id, migration_key, version)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT(account_id, migration_key) DO UPDATE SET version = excluded.version",
                params![account_id, FACTS_MIGRATION, DASHBOARD_FACTS_VERSION],
            )
            .map_err(|error| format!("保存面板数据版本失败：{error}"))?;
    }
    Ok(())
}

fn backfill(
    connection: &mut Connection,
    account_id: &str,
    record_type: &str,
) -> Result<(), String> {
    let fact_table = match record_type {
        "battle" => "extension_dashboard_battles",
        _ => "extension_dashboard_coop",
    };
    let missing_sql = format!(
        "SELECT r.rowid, r.record_json, r.raw_json, r.raw_encoding
         FROM history_records r
         LEFT JOIN {fact_table} f ON f.account_id = r.account_id AND f.record_id = r.record_id
         WHERE r.account_id = ?1 AND r.record_type = ?2 AND f.record_id IS NULL AND r.rowid > ?3
         ORDER BY r.rowid
         LIMIT ?4"
    );
    let mut after_rowid = 0_i64;
    loop {
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| format!("开启面板数据回填事务失败：{error}"))?;
        let rows = {
            let mut statement = transaction
                .prepare(&missing_sql)
                .map_err(|error| format!("准备面板数据回填查询失败：{error}"))?;
            let rows = statement
                .query_map(
                    params![account_id, record_type, after_rowid, BACKFILL_BATCH_SIZE],
                    |row| {
                        Ok((
                            row.get::<_, i64>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, Vec<u8>>(2)?,
                            row.get::<_, String>(3)?,
                        ))
                    },
                )
                .map_err(|error| format!("读取待回填的历史记录失败：{error}"))?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|error| format!("读取待回填的历史记录行失败：{error}"))?
        };
        if rows.is_empty() {
            return Ok(());
        }
        for (rowid, record_json, raw_json, raw_encoding) in rows {
            after_rowid = rowid;
            // A record whose raw JSON cannot be read still gets the fields
            // that the archived parse provides.
            let raw =
                history_store::decode_raw_json(&raw_json, &raw_encoding).unwrap_or(Value::Null);
            if record_type == "battle" {
                if let Ok(record) = serde_json::from_str::<BattleRecord>(&record_json) {
                    upsert_battle(&transaction, account_id, &record, &raw)?;
                }
            } else if let Ok(record) = serde_json::from_str::<CoopRecord>(&record_json) {
                upsert_coop(&transaction, account_id, &record, &raw)?;
            }
        }
        transaction
            .commit()
            .map_err(|error| format!("提交面板数据回填失败：{error}"))?;
    }
}

#[derive(Debug, Serialize)]
pub(crate) struct DashboardFacts {
    pub(crate) account_id: String,
    pub(crate) fetched_at_ms: Option<i64>,
    pub(crate) battles: Vec<BattleRow>,
    pub(crate) coop: Vec<CoopRow>,
}

#[derive(Debug, Serialize)]
pub(crate) struct BattleRow {
    id: String,
    played_at_ms: i64,
    mode: String,
    bankara_mode: Option<String>,
    rule_code: Option<String>,
    rule_name: Option<String>,
    stage_id: Option<String>,
    stage_name: Option<String>,
    weapon_id: Option<String>,
    weapon_name: Option<String>,
    weapon_image: Option<String>,
    judgement: Option<String>,
    knockout: Option<String>,
    duration_seconds: Option<i64>,
    kills: Option<i64>,
    assists: Option<i64>,
    deaths: Option<i64>,
    specials: Option<i64>,
    paint: Option<i64>,
    power: Option<f64>,
    power_kind: Option<String>,
}

#[derive(Debug, Serialize)]
pub(crate) struct CoopRow {
    id: String,
    played_at_ms: i64,
    rule: Option<String>,
    stage_id: Option<String>,
    stage_name: Option<String>,
    result_wave: Option<i64>,
    cleared: Option<bool>,
    waves: Vec<CoopWaveFact>,
    boss_appeared: bool,
    boss_defeated: Option<bool>,
    boss_id: Option<String>,
    boss_name: Option<String>,
    danger_rate: Option<f64>,
    grade_name: Option<String>,
    grade_point: Option<i64>,
    golden_eggs: Option<i64>,
    power_eggs: Option<i64>,
    rescue_count: Option<i64>,
    rescued_count: Option<i64>,
    defeat_enemy_count: Option<i64>,
    scale_gold: Option<i64>,
    scale_silver: Option<i64>,
    scale_bronze: Option<i64>,
}

/// Returns every fact with a known play time, oldest first.
pub(crate) fn load_facts(history_dir: &Path, account_id: &str) -> Result<DashboardFacts, String> {
    ensure_facts(history_dir, account_id)?;
    let connection = history_store::open_database(history_dir)?;
    let fetched_at_ms = connection
        .query_row(
            "SELECT fetched_at_ms FROM history_meta WHERE account_id = ?1",
            [account_id],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map_err(|error| format!("读取历史同步时间失败：{error}"))?;

    let battles = {
        let mut statement = connection
            .prepare(
                "SELECT record_id, played_at_ms, mode, bankara_mode, rule_code, rule_name,
                        stage_id, stage_name, weapon_id, weapon_name, judgement, knockout,
                        duration_seconds, kills, assists, deaths, specials, paint, power, power_kind,
                        weapon_image
                 FROM extension_dashboard_battles
                 WHERE account_id = ?1 AND played_at_ms IS NOT NULL
                 ORDER BY played_at_ms, record_id",
            )
            .map_err(|error| format!("准备面板对战查询失败：{error}"))?;
        let rows = statement
            .query_map([account_id], |row| {
                Ok(BattleRow {
                    id: row.get(0)?,
                    played_at_ms: row.get(1)?,
                    mode: row.get(2)?,
                    bankara_mode: row.get(3)?,
                    rule_code: row.get(4)?,
                    rule_name: row.get(5)?,
                    stage_id: row.get(6)?,
                    stage_name: row.get(7)?,
                    weapon_id: row.get(8)?,
                    weapon_name: row.get(9)?,
                    judgement: row.get(10)?,
                    knockout: row.get(11)?,
                    duration_seconds: row.get(12)?,
                    kills: row.get(13)?,
                    assists: row.get(14)?,
                    deaths: row.get(15)?,
                    specials: row.get(16)?,
                    paint: row.get(17)?,
                    power: row.get(18)?,
                    power_kind: row.get(19)?,
                    weapon_image: row.get(20)?,
                })
            })
            .map_err(|error| format!("读取面板对战数据失败：{error}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("读取面板对战数据行失败：{error}"))?
    };

    let coop = {
        let mut statement = connection
            .prepare(
                "SELECT record_id, played_at_ms, rule, stage_id, stage_name, result_wave, cleared,
                        waves_json, boss_appeared, boss_defeated, boss_id, boss_name, danger_rate,
                        grade_name, grade_point, golden_eggs, power_eggs, rescue_count,
                        rescued_count, defeat_enemy_count, scale_gold, scale_silver, scale_bronze
                 FROM extension_dashboard_coop
                 WHERE account_id = ?1 AND played_at_ms IS NOT NULL
                 ORDER BY played_at_ms, record_id",
            )
            .map_err(|error| format!("准备面板打工查询失败：{error}"))?;
        let rows = statement
            .query_map([account_id], |row| {
                let waves_json: String = row.get(7)?;
                Ok(CoopRow {
                    id: row.get(0)?,
                    played_at_ms: row.get(1)?,
                    rule: row.get(2)?,
                    stage_id: row.get(3)?,
                    stage_name: row.get(4)?,
                    result_wave: row.get(5)?,
                    cleared: row.get(6)?,
                    waves: serde_json::from_str(&waves_json).unwrap_or_default(),
                    boss_appeared: row.get(8)?,
                    boss_defeated: row.get(9)?,
                    boss_id: row.get(10)?,
                    boss_name: row.get(11)?,
                    danger_rate: row.get(12)?,
                    grade_name: row.get(13)?,
                    grade_point: row.get(14)?,
                    golden_eggs: row.get(15)?,
                    power_eggs: row.get(16)?,
                    rescue_count: row.get(17)?,
                    rescued_count: row.get(18)?,
                    defeat_enemy_count: row.get(19)?,
                    scale_gold: row.get(20)?,
                    scale_silver: row.get(21)?,
                    scale_bronze: row.get(22)?,
                })
            })
            .map_err(|error| format!("读取面板打工数据失败：{error}"))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("读取面板打工数据行失败：{error}"))?
    };

    Ok(DashboardFacts {
        account_id: account_id.to_owned(),
        fetched_at_ms,
        battles,
        coop,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use splat3_widget_core::history::{ArchivedBattle, ArchivedCoop, HistoryFetchedRecord};
    use tempfile::tempdir;

    fn battle(id: &str, played_time: &str, power: Option<f64>) -> ArchivedBattle {
        ArchivedBattle {
            record: BattleRecord {
                id: id.to_owned(),
                played_time: Some(played_time.to_owned()),
                mode: "X_MATCH".to_owned(),
                bankara_mode: None,
                rule: Some("真格区域".to_owned()),
                stage: Some("鳗鲶区".to_owned()),
                judgement: Some("WIN".to_owned()),
                player_name: None,
                weapon: Some("斯普拉机动枪".to_owned()),
                paint: Some(980),
                power,
                power_kind: Some("x".to_owned()),
                kills: Some(6),
                assists: Some(1),
                deaths: Some(3),
                specials: Some(2),
            },
            raw: json!({
                "id": id,
                "vsRule": { "rule": "AREA" },
                "vsStage": { "id": "stage-1" },
                "duration": 300,
                "myTeam": { "players": [{ "isMyself": true, "weapon": { "id": "weapon-40" } }] }
            }),
        }
    }

    fn coop(id: &str, played_time: &str) -> ArchivedCoop {
        ArchivedCoop {
            record: CoopRecord {
                id: id.to_owned(),
                played_time: Some(played_time.to_owned()),
                rule: Some("REGULAR".to_owned()),
                stage: Some("鲑坝".to_owned()),
                result_wave: Some(0),
                clear_waves: Some(3),
                job_point: None,
                after_grade: Some("达人".to_owned()),
                after_grade_point: Some(400.0),
                danger_rate: Some(2.0),
                golden_eggs: None,
                power_eggs: None,
                rescued: None,
                rescued_by_others: None,
                rotation_start_time: None,
                rotation_end_time: None,
            },
            raw: json!({
                "id": id,
                "resultWave": 0,
                "waveResults": [{ "waveNumber": 1, "waterLevel": 1, "deliverNorm": 18 }],
                "myResult": { "goldenDeliverCount": 12 }
            }),
        }
    }

    fn fact_count(history_dir: &Path, table: &str, account_id: &str) -> i64 {
        history_store::open_database(history_dir)
            .unwrap()
            .query_row(
                &format!("SELECT COUNT(*) FROM {table} WHERE account_id = ?1"),
                [account_id],
                |row| row.get(0),
            )
            .unwrap()
    }

    fn raw_bytes(history_dir: &Path, record_id: &str) -> Vec<u8> {
        history_store::open_database(history_dir)
            .unwrap()
            .query_row(
                "SELECT raw_json FROM history_records WHERE record_id = ?1",
                [record_id],
                |row| row.get(0),
            )
            .unwrap()
    }

    #[test]
    fn upgrades_an_existing_battle_fact_table_with_the_weapon_image_column() {
        let directory = tempdir().unwrap();
        std::fs::create_dir_all(directory.path()).unwrap();
        let connection =
            rusqlite::Connection::open(directory.path().join("history.sqlite3")).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE extension_dashboard_battles (
                     account_id TEXT NOT NULL,
                     record_id TEXT NOT NULL,
                     played_at_ms INTEGER,
                     mode TEXT NOT NULL,
                     bankara_mode TEXT, rule_code TEXT, rule_name TEXT, stage_id TEXT, stage_name TEXT,
                     weapon_id TEXT, weapon_name TEXT, judgement TEXT, knockout TEXT,
                     duration_seconds INTEGER, kills INTEGER, assists INTEGER, deaths INTEGER,
                     specials INTEGER, paint INTEGER, power REAL, power_kind TEXT,
                     PRIMARY KEY (account_id, record_id)
                 );",
            )
            .unwrap();
        drop(connection);

        let connection = history_store::open_database(directory.path()).unwrap();
        let columns: Vec<String> = connection
            .prepare("PRAGMA table_info(extension_dashboard_battles)")
            .unwrap()
            .query_map([], |row| row.get(1))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert!(columns.iter().any(|column| column == "weapon_image"));
        // 再打开一次不会重复加列。
        drop(connection);
        history_store::open_database(directory.path()).unwrap();
    }

    #[test]
    fn writer_saves_facts_in_the_same_transaction_as_the_archive() {
        let directory = tempdir().unwrap();
        let mut writer = history_store::HistoryRecordWriter::open(directory.path()).unwrap();
        writer
            .save(
                "account",
                1,
                HistoryFetchedRecord::Battle(battle("b1", "2026-10-16T13:00:00Z", Some(2100.0))),
            )
            .unwrap();
        writer
            .save(
                "account",
                1,
                HistoryFetchedRecord::Coop(coop("c1", "2026-10-16T12:00:00Z")),
            )
            .unwrap();

        let facts = load_facts(directory.path(), "account").unwrap();
        assert_eq!(facts.battles.len(), 1);
        let row = &facts.battles[0];
        assert_eq!(row.played_at_ms, 1_792_155_600_000);
        assert_eq!(row.rule_code.as_deref(), Some("AREA"));
        assert_eq!(row.weapon_id.as_deref(), Some("weapon-40"));
        assert_eq!(row.duration_seconds, Some(300));
        assert_eq!(facts.coop.len(), 1);
        assert_eq!(facts.coop[0].golden_eggs, Some(12));
        assert_eq!(facts.coop[0].cleared, Some(true));
        assert_eq!(facts.coop[0].waves.len(), 1);
    }

    #[test]
    fn backfill_indexes_older_archives_without_touching_raw_json() {
        let directory = tempdir().unwrap();
        let cache = splat3_widget_core::history::HistoryCache {
            schema_version: 1,
            fetched_at_ms: 5,
            battles: vec![
                battle("b1", "2026-10-15T10:00:00Z", Some(2000.0)),
                battle("b2", "2026-10-16T10:00:00Z", Some(2010.0)),
            ],
            coop: vec![coop("c1", "2026-10-16T09:00:00Z")],
        };
        history_store::save_cache(directory.path(), "account", &cache).unwrap();
        let before = raw_bytes(directory.path(), "b1");
        {
            // Simulate an archive written before the dashboard existed.
            let connection = history_store::open_database(directory.path()).unwrap();
            connection
                .execute_batch(
                    "DELETE FROM extension_dashboard_battles;
                     DELETE FROM extension_dashboard_coop;",
                )
                .unwrap();
        }
        assert_eq!(
            fact_count(directory.path(), "extension_dashboard_battles", "account"),
            0
        );

        let facts = load_facts(directory.path(), "account").unwrap();
        assert_eq!(facts.battles.len(), 2);
        assert_eq!(facts.battles[0].id, "b1");
        assert_eq!(facts.battles[1].id, "b2");
        assert_eq!(facts.coop.len(), 1);
        assert_eq!(facts.fetched_at_ms, Some(5));
        assert_eq!(raw_bytes(directory.path(), "b1"), before);
    }

    #[test]
    fn a_new_extraction_version_rebuilds_stale_facts() {
        let directory = tempdir().unwrap();
        let mut writer = history_store::HistoryRecordWriter::open(directory.path()).unwrap();
        writer
            .save(
                "account",
                1,
                HistoryFetchedRecord::Battle(battle("b1", "2026-10-16T13:00:00Z", Some(2100.0))),
            )
            .unwrap();
        ensure_facts(directory.path(), "account").unwrap();
        {
            let connection = history_store::open_database(directory.path()).unwrap();
            connection
                .execute_batch(
                    "UPDATE extension_dashboard_battles SET rule_code = 'STALE';
                     UPDATE history_migrations SET version = 0
                     WHERE migration_key = 'dashboard-facts';",
                )
                .unwrap();
        }
        let facts = load_facts(directory.path(), "account").unwrap();
        assert_eq!(facts.battles[0].rule_code.as_deref(), Some("AREA"));
    }

    #[test]
    fn x_settlement_updates_the_fact_power() {
        let directory = tempdir().unwrap();
        let mut writer = history_store::HistoryRecordWriter::open(directory.path()).unwrap();
        writer
            .save(
                "account",
                1,
                HistoryFetchedRecord::Battle(battle("b1", "2026-10-16T13:00:00Z", Some(2100.0))),
            )
            .unwrap();
        writer
            .save(
                "account",
                2,
                HistoryFetchedRecord::XBattleSettlement {
                    record_id: "b1".to_owned(),
                    power_after: 2123.4,
                    payload_hash: "hash".to_owned(),
                    raw_group: json!({ "group": 1 }),
                },
            )
            .unwrap();
        let facts = load_facts(directory.path(), "account").unwrap();
        assert_eq!(facts.battles[0].power, Some(2123.4));
    }

    #[test]
    fn deleting_an_account_removes_only_its_facts() {
        let directory = tempdir().unwrap();
        let mut writer = history_store::HistoryRecordWriter::open(directory.path()).unwrap();
        for account in ["first", "second"] {
            writer
                .save(
                    account,
                    1,
                    HistoryFetchedRecord::Battle(battle("b1", "2026-10-16T13:00:00Z", None)),
                )
                .unwrap();
            writer
                .save(
                    account,
                    1,
                    HistoryFetchedRecord::Coop(coop("c1", "2026-10-16T12:00:00Z")),
                )
                .unwrap();
        }
        drop(writer);
        history_store::delete_account(directory.path(), "first").unwrap();
        for table in ["extension_dashboard_battles", "extension_dashboard_coop"] {
            assert_eq!(fact_count(directory.path(), table, "first"), 0);
            assert_eq!(fact_count(directory.path(), table, "second"), 1);
        }
    }

    #[test]
    fn facts_without_a_play_time_are_kept_out_of_the_timeline() {
        let directory = tempdir().unwrap();
        let mut writer = history_store::HistoryRecordWriter::open(directory.path()).unwrap();
        let mut undated = battle("b0", "2026-10-16T13:00:00Z", None);
        undated.record.played_time = None;
        writer
            .save("account", 1, HistoryFetchedRecord::Battle(undated))
            .unwrap();
        writer
            .save(
                "account",
                1,
                HistoryFetchedRecord::Battle(battle("b1", "2026-10-16T13:00:00Z", None)),
            )
            .unwrap();
        let facts = load_facts(directory.path(), "account").unwrap();
        assert_eq!(facts.battles.len(), 1);
        assert_eq!(facts.battles[0].id, "b1");
        assert_eq!(
            fact_count(directory.path(), "extension_dashboard_battles", "account"),
            2
        );
    }
}
