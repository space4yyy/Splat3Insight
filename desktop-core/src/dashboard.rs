//! Flat per-record facts for the dashboard.
//!
//! Facts are derived from the archived parse and the original SplatNet JSON.
//! They are a disposable index: the archive keeps the raw JSON untouched, and
//! the facts can be rebuilt whenever this extraction changes.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::history::{BattleRecord, CoopRecord};

/// Bump when the extraction below changes so stored facts are rebuilt.
pub const DASHBOARD_FACTS_VERSION: i64 = 2;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BattleFact {
    pub record_id: String,
    pub played_time: Option<String>,
    pub mode: String,
    pub bankara_mode: Option<String>,
    pub rule_code: Option<String>,
    pub rule_name: Option<String>,
    pub stage_id: Option<String>,
    pub stage_name: Option<String>,
    pub weapon_id: Option<String>,
    pub weapon_name: Option<String>,
    /// 我这场用的武器图片（带签名的原链接，桌面端按图片本身缓存）。
    pub weapon_image: Option<String>,
    pub judgement: Option<String>,
    pub knockout: Option<String>,
    pub duration_seconds: Option<i64>,
    pub kills: Option<i64>,
    pub assists: Option<i64>,
    pub deaths: Option<i64>,
    pub specials: Option<i64>,
    pub paint: Option<i64>,
    pub power: Option<f64>,
    pub power_kind: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoopWaveFact {
    pub wave_number: i64,
    pub water_level: Option<i64>,
    pub event_wave: Option<String>,
    pub boss_wave: bool,
    pub cleared: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CoopFact {
    pub record_id: String,
    pub played_time: Option<String>,
    pub rule: Option<String>,
    pub stage_id: Option<String>,
    pub stage_name: Option<String>,
    pub result_wave: Option<i64>,
    pub cleared: Option<bool>,
    pub waves: Vec<CoopWaveFact>,
    pub boss_appeared: bool,
    pub boss_defeated: Option<bool>,
    pub boss_id: Option<String>,
    pub boss_name: Option<String>,
    pub danger_rate: Option<f64>,
    pub grade_name: Option<String>,
    pub grade_point: Option<i64>,
    pub golden_eggs: Option<i64>,
    pub power_eggs: Option<i64>,
    pub rescue_count: Option<i64>,
    pub rescued_count: Option<i64>,
    pub defeat_enemy_count: Option<i64>,
    pub scale_gold: Option<i64>,
    pub scale_silver: Option<i64>,
    pub scale_bronze: Option<i64>,
}

pub fn battle_fact(record: &BattleRecord, raw: &Value) -> BattleFact {
    let player = my_battle_player(raw);
    BattleFact {
        record_id: record.id.clone(),
        played_time: record.played_time.clone(),
        mode: record.mode.clone(),
        bankara_mode: record.bankara_mode.clone(),
        rule_code: text(raw, "/vsRule/rule"),
        rule_name: record.rule.clone(),
        stage_id: text(raw, "/vsStage/id"),
        stage_name: record.stage.clone(),
        weapon_id: player.and_then(|player| text(player, "/weapon/id")),
        weapon_name: record.weapon.clone(),
        weapon_image: player.and_then(|player| text(player, "/weapon/image/url")),
        judgement: record.judgement.clone(),
        knockout: text(raw, "/knockout"),
        duration_seconds: integer(raw, "/duration"),
        kills: record.kills,
        assists: record.assists,
        deaths: record.deaths,
        specials: record.specials,
        paint: record.paint,
        power: record.power,
        power_kind: record.power_kind.clone(),
    }
}

pub fn coop_fact(record: &CoopRecord, raw: &Value) -> CoopFact {
    let result_wave = record.result_wave.or_else(|| integer(raw, "/resultWave"));
    let boss_result = raw.get("bossResult").filter(|value| value.is_object());
    let boss_defeated = boss_result.and_then(|result| result.get("hasDefeatBoss")?.as_bool());
    let boss = boss_result
        .and_then(|result| result.get("boss"))
        .filter(|value| value.is_object())
        .or_else(|| raw.get("boss").filter(|value| value.is_object()));
    let my_result = raw.get("myResult");
    let my_count = |keys: &[&str]| {
        my_result.and_then(|result| keys.iter().find_map(|key| result.get(*key)?.as_i64()))
    };
    let scale = raw.get("scale").filter(|value| value.is_object());
    let scale_count = |key: &str| scale.and_then(|scale| scale.get(key)?.as_i64());

    CoopFact {
        record_id: record.id.clone(),
        played_time: record.played_time.clone(),
        rule: record.rule.clone(),
        stage_id: text(raw, "/coopStage/id"),
        stage_name: record.stage.clone(),
        result_wave,
        cleared: result_wave.map(|wave| wave == 0),
        waves: coop_waves(raw, result_wave, boss_result.is_some(), boss_defeated),
        boss_appeared: boss_result.is_some(),
        boss_defeated,
        boss_id: boss.and_then(|boss| text(boss, "/id")),
        boss_name: boss.and_then(|boss| text(boss, "/name")),
        danger_rate: record.danger_rate,
        grade_name: record.after_grade.clone(),
        grade_point: record
            .after_grade_point
            .filter(|point| point.fract() == 0.0)
            .map(|point| point as i64),
        golden_eggs: my_count(&["goldenDeliverCount", "goldenEggs", "goldenEgg"])
            .or(record.golden_eggs),
        power_eggs: my_count(&["deliverCount", "powerEggs", "powerEgg"]).or(record.power_eggs),
        rescue_count: my_count(&["rescueCount", "rescuedByOthers"]).or(record.rescued_by_others),
        rescued_count: my_count(&["rescuedCount", "rescued"]).or(record.rescued),
        defeat_enemy_count: my_count(&["defeatEnemyCount"]),
        scale_gold: scale_count("gold"),
        scale_silver: scale_count("silver"),
        scale_bronze: scale_count("bronze"),
    }
}

fn coop_waves(
    raw: &Value,
    result_wave: Option<i64>,
    boss_appeared: bool,
    boss_defeated: Option<bool>,
) -> Vec<CoopWaveFact> {
    let Some(waves) = raw.get("waveResults").and_then(Value::as_array) else {
        return Vec::new();
    };
    waves
        .iter()
        .filter_map(|wave| {
            let wave_number = wave.get("waveNumber")?.as_i64()?;
            // The extra wave against a King Salmonid has no egg quota.
            let boss_wave = boss_appeared && wave.get("deliverNorm").is_none_or(Value::is_null);
            let cleared = if boss_wave {
                boss_defeated
            } else {
                result_wave.map(|failed_wave| failed_wave == 0 || wave_number < failed_wave)
            };
            Some(CoopWaveFact {
                wave_number,
                water_level: wave.get("waterLevel").and_then(Value::as_i64),
                event_wave: text(wave, "/eventWave/name"),
                boss_wave,
                cleared,
            })
        })
        .collect()
}

fn my_battle_player(raw: &Value) -> Option<&Value> {
    let players = raw.pointer("/myTeam/players")?.as_array()?;
    players
        .iter()
        .find(|player| player.get("isMyself").and_then(Value::as_bool) == Some(true))
        .or_else(|| players.first())
}

fn text(value: &Value, pointer: &str) -> Option<String> {
    value
        .pointer(pointer)?
        .as_str()
        .filter(|text| !text.is_empty())
        .map(ToOwned::to_owned)
}

fn integer(value: &Value, pointer: &str) -> Option<i64> {
    let value = value.pointer(pointer)?;
    value.as_i64().or_else(|| {
        value
            .as_f64()
            .filter(|number| number.fract() == 0.0)
            .map(|number| number as i64)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn battle_record() -> BattleRecord {
        BattleRecord {
            id: "battle-1".to_owned(),
            played_time: Some("2026-10-16T13:40:00Z".to_owned()),
            mode: "BANKARA".to_owned(),
            bankara_mode: Some("CHALLENGE".to_owned()),
            rule: Some("真格区域".to_owned()),
            stage: Some("鳗鲶区".to_owned()),
            judgement: Some("WIN".to_owned()),
            player_name: Some("墨墨鱼".to_owned()),
            weapon: Some("斯普拉机动枪".to_owned()),
            paint: Some(1024),
            power: Some(1987.4),
            power_kind: Some("weapon".to_owned()),
            kills: Some(7),
            assists: Some(2),
            deaths: Some(4),
            specials: Some(3),
        }
    }

    fn coop_record(result_wave: Option<i64>) -> CoopRecord {
        CoopRecord {
            id: "coop-1".to_owned(),
            played_time: Some("2026-10-16T11:05:00Z".to_owned()),
            rule: Some("REGULAR".to_owned()),
            stage: Some("鲑坝".to_owned()),
            result_wave,
            clear_waves: None,
            job_point: None,
            after_grade: Some("达人".to_owned()),
            after_grade_point: Some(640.0),
            danger_rate: Some(2.4),
            golden_eggs: None,
            power_eggs: None,
            rescued: Some(1),
            rescued_by_others: Some(2),
            rotation_start_time: None,
            rotation_end_time: None,
        }
    }

    #[test]
    fn battle_fact_reads_codes_ids_and_timing_from_raw_json() {
        let raw = json!({
            "vsRule": { "rule": "AREA", "name": "真格区域" },
            "vsStage": { "id": "VnNTdGFnZS0x", "name": "鳗鲶区" },
            "knockout": "WIN",
            "duration": 214,
            "myTeam": { "players": [
                { "isMyself": false, "weapon": { "id": "other" } },
                { "isMyself": true, "weapon": { "id": "V2VhcG9uLTQw", "name": "斯普拉机动枪", "image": { "url": "https://example.test/w.png" } } }
            ] }
        });
        let fact = battle_fact(&battle_record(), &raw);
        assert_eq!(fact.rule_code.as_deref(), Some("AREA"));
        assert_eq!(fact.rule_name.as_deref(), Some("真格区域"));
        assert_eq!(fact.stage_id.as_deref(), Some("VnNTdGFnZS0x"));
        assert_eq!(fact.weapon_id.as_deref(), Some("V2VhcG9uLTQw"));
        assert_eq!(
            fact.weapon_image.as_deref(),
            Some("https://example.test/w.png")
        );
        assert_eq!(fact.knockout.as_deref(), Some("WIN"));
        assert_eq!(fact.duration_seconds, Some(214));
        assert_eq!(fact.power, Some(1987.4));
        assert_eq!(fact.power_kind.as_deref(), Some("weapon"));
        assert_eq!(fact.kills, Some(7));
    }

    #[test]
    fn battle_fact_leaves_missing_raw_fields_empty() {
        let fact = battle_fact(&battle_record(), &json!({ "id": "battle-1" }));
        assert_eq!(fact.rule_code, None);
        assert_eq!(fact.stage_id, None);
        assert_eq!(fact.weapon_id, None);
        assert_eq!(fact.knockout, None);
        assert_eq!(fact.duration_seconds, None);
        assert_eq!(fact.rule_name.as_deref(), Some("真格区域"));
    }

    #[test]
    fn cleared_coop_fact_marks_every_wave_and_the_king_wave() {
        let raw = json!({
            "coopStage": { "id": "Q29vcFN0YWdlLTE=", "name": "鲑坝" },
            "resultWave": 0,
            "waveResults": [
                { "waveNumber": 1, "waterLevel": 1, "deliverNorm": 18, "eventWave": null },
                { "waveNumber": 2, "waterLevel": 2, "deliverNorm": 20, "eventWave": { "name": "大量出现" } },
                { "waveNumber": 3, "waterLevel": 0, "deliverNorm": 22, "eventWave": null },
                { "waveNumber": 4, "waterLevel": 1, "deliverNorm": null, "eventWave": null }
            ],
            "bossResult": { "hasDefeatBoss": true, "boss": { "id": "Q29vcEVuZW15LTIz", "name": "横纲" } },
            "scale": { "gold": 1, "silver": 2, "bronze": 3 },
            "myResult": {
                "goldenDeliverCount": 14,
                "deliverCount": 1203,
                "rescueCount": 3,
                "rescuedCount": 1,
                "defeatEnemyCount": 11
            }
        });
        let fact = coop_fact(&coop_record(Some(0)), &raw);
        assert_eq!(fact.cleared, Some(true));
        assert_eq!(fact.waves.len(), 4);
        assert!(fact.waves[..3]
            .iter()
            .all(|wave| wave.cleared == Some(true) && !wave.boss_wave));
        assert_eq!(fact.waves[1].event_wave.as_deref(), Some("大量出现"));
        assert_eq!(fact.waves[1].water_level, Some(2));
        assert!(fact.waves[3].boss_wave);
        assert_eq!(fact.waves[3].cleared, Some(true));
        assert!(fact.boss_appeared);
        assert_eq!(fact.boss_defeated, Some(true));
        assert_eq!(fact.boss_id.as_deref(), Some("Q29vcEVuZW15LTIz"));
        assert_eq!(fact.golden_eggs, Some(14));
        assert_eq!(fact.power_eggs, Some(1203));
        assert_eq!(fact.rescue_count, Some(3));
        assert_eq!(fact.rescued_count, Some(1));
        assert_eq!(fact.defeat_enemy_count, Some(11));
        assert_eq!(
            (fact.scale_gold, fact.scale_silver, fact.scale_bronze),
            (Some(1), Some(2), Some(3))
        );
        assert_eq!(fact.grade_point, Some(640));
    }

    #[test]
    fn failed_coop_fact_marks_the_failed_wave_and_keeps_missing_counts_from_the_record() {
        let raw = json!({
            "resultWave": 2,
            "waveResults": [
                { "waveNumber": 1, "waterLevel": 0, "deliverNorm": 17 },
                { "waveNumber": 2, "waterLevel": 2, "deliverNorm": 19 }
            ],
            "bossResult": null,
            "scale": null
        });
        let fact = coop_fact(&coop_record(Some(2)), &raw);
        assert_eq!(fact.cleared, Some(false));
        assert_eq!(fact.waves[0].cleared, Some(true));
        assert_eq!(fact.waves[1].cleared, Some(false));
        assert!(!fact.boss_appeared);
        assert_eq!(fact.boss_defeated, None);
        assert_eq!(fact.scale_gold, None);
        assert_eq!(fact.golden_eggs, None);
        assert_eq!(fact.rescue_count, Some(2));
        assert_eq!(fact.rescued_count, Some(1));
    }
}
