//! Data adapter for the compact B-style overlay card.

use serde::{Deserialize, Serialize};

use crate::history::{ArchivedBattle, BattleRecord, HistoryCache};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OverlayState {
    pub selected: Option<OverlayBattle>,
    pub recent_summary: RecentSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OverlayBattle {
    pub id: String,
    pub played_time: Option<String>,
    pub mode: String,
    pub bankara_mode: Option<String>,
    pub rule: Option<String>,
    pub judgement: Option<String>,
    pub power: Option<f64>,
    pub power_kind: Option<String>,
    pub power_delta: Option<f64>,
    pub kills: Option<i64>,
    pub assists: Option<i64>,
    pub deaths: Option<i64>,
    pub specials: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RecentSummary {
    pub sample_size: usize,
    pub win: usize,
    pub lose: usize,
    pub win_rate: f64,
}

impl OverlayState {
    pub fn from_history(cache: &HistoryCache) -> Self {
        let mut battles: Vec<&ArchivedBattle> = cache
            .battles
            .iter()
            .filter(|item| is_overlay_battle(&item.record))
            .collect();
        battles.sort_by(|left, right| right.record.played_time.cmp(&left.record.played_time));

        let recent = battles.iter().take(50);
        let win = recent
            .clone()
            .filter(|item| item.record.judgement.as_deref() == Some("WIN"))
            .count();
        let lose = recent
            .filter(|item| {
                matches!(
                    item.record.judgement.as_deref(),
                    Some("LOSE") | Some("DEEMED_LOSE") | Some("EXEMPTED_LOSE")
                )
            })
            .count();
        let sample_size = win + lose;

        Self {
            selected: battles
                .first()
                .map(|item| to_overlay_battle(item, &battles[1..])),
            recent_summary: RecentSummary {
                sample_size,
                win,
                lose,
                win_rate: if sample_size == 0 {
                    0.0
                } else {
                    win as f64 / sample_size as f64 * 100.0
                },
            },
        }
    }
}

fn is_overlay_battle(record: &BattleRecord) -> bool {
    matches!(record.mode.as_str(), "X_MATCH" | "BANKARA")
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PowerTrack {
    XMatch {
        power_kind: String,
        rule: String,
    },
    AnarchyChallenge {
        power_kind: String,
        weapon_id: String,
    },
    AnarchyOpen {
        power_kind: String,
        player_ids: Vec<String>,
    },
}

fn power_track(item: &ArchivedBattle) -> Option<PowerTrack> {
    let record = &item.record;
    let power_kind = record.power_kind.as_ref()?.clone();
    match (record.mode.as_str(), record.bankara_mode.as_deref()) {
        ("X_MATCH", _) => item
            .raw
            .pointer("/vsRule/rule")
            .and_then(serde_json::Value::as_str)
            .or(record.rule.as_deref())
            .filter(|rule| !rule.is_empty())
            .map(|rule| PowerTrack::XMatch {
                power_kind,
                rule: rule.to_owned(),
            }),
        ("BANKARA", Some("CHALLENGE")) => {
            let players = item.raw.pointer("/myTeam/players")?.as_array()?;
            let player = players
                .iter()
                .find(|player| {
                    player.get("isMyself").and_then(serde_json::Value::as_bool) == Some(true)
                })
                .or_else(|| players.first())?;
            let weapon_id = player
                .pointer("/weapon/id")
                .and_then(serde_json::Value::as_str)
                .filter(|id| !id.is_empty())?;
            Some(PowerTrack::AnarchyChallenge {
                power_kind,
                weapon_id: weapon_id.to_owned(),
            })
        }
        ("BANKARA", Some("OPEN")) => {
            let players = item.raw.pointer("/myTeam/players")?.as_array()?;
            if players.is_empty() {
                return None;
            }
            let mut player_ids = players
                .iter()
                .map(|player| {
                    player
                        .get("id")
                        .and_then(serde_json::Value::as_str)
                        .filter(|id| !id.is_empty())
                        .or_else(|| {
                            player
                                .get("nameId")
                                .and_then(serde_json::Value::as_str)
                                .filter(|id| !id.is_empty())
                        })
                        .map(str::to_owned)
                })
                .collect::<Option<Vec<_>>>()?;
            player_ids.sort_unstable();
            Some(PowerTrack::AnarchyOpen {
                power_kind,
                player_ids,
            })
        }
        _ => None,
    }
}

fn to_overlay_battle(item: &ArchivedBattle, older: &[&ArchivedBattle]) -> OverlayBattle {
    let current_track = power_track(item);
    let power_delta = item.record.power.and_then(|current_power| {
        let track = current_track.as_ref()?;
        Some(
            older
                .iter()
                .filter(|candidate| {
                    candidate.record.power.is_some()
                        && power_track(candidate).as_ref() == Some(track)
                        && (candidate.record.power.unwrap() - current_power).abs() > 0.0001
                })
                .find_map(|candidate| candidate.record.power.map(|power| current_power - power))
                // A newly rated track starts at +0; an unplaced track has no current power,
                // so the outer `and_then` keeps both values absent until a rating exists.
                .unwrap_or(0.0),
        )
    });

    OverlayBattle {
        id: item.record.id.clone(),
        played_time: item.record.played_time.clone(),
        mode: item.record.mode.clone(),
        bankara_mode: item.record.bankara_mode.clone(),
        rule: item.record.rule.clone(),
        judgement: item.record.judgement.clone(),
        power: item.record.power,
        power_kind: item.record.power_kind.clone(),
        power_delta,
        kills: item.record.kills,
        assists: item.record.assists,
        deaths: item.record.deaths,
        specials: item.record.specials,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn battle(
        id: &str,
        played_time: &str,
        mode: &str,
        power: Option<f64>,
        judgement: &str,
    ) -> ArchivedBattle {
        ArchivedBattle {
            record: BattleRecord {
                id: id.to_owned(),
                played_time: Some(played_time.to_owned()),
                mode: mode.to_owned(),
                bankara_mode: None,
                rule: Some("Clam Blitz".to_owned()),
                stage: None,
                judgement: Some(judgement.to_owned()),
                player_name: None,
                weapon: None,
                paint: None,
                power,
                power_kind: power.map(|_| "x".to_owned()),
                kills: Some(9),
                assists: Some(2),
                deaths: Some(5),
                specials: None,
            },
            raw: serde_json::json!({ "id": id }),
        }
    }

    #[test]
    fn builds_card_state_and_keeps_the_last_power_change() {
        let cache = HistoryCache {
            schema_version: 1,
            fetched_at_ms: 1,
            battles: vec![
                battle(
                    "latest",
                    "2026-09-22T03:00:00Z",
                    "X_MATCH",
                    Some(2020.0),
                    "WIN",
                ),
                battle(
                    "same",
                    "2026-09-22T02:00:00Z",
                    "X_MATCH",
                    Some(2020.0),
                    "LOSE",
                ),
                battle(
                    "changed",
                    "2026-09-22T01:00:00Z",
                    "X_MATCH",
                    Some(2000.0),
                    "WIN",
                ),
            ],
            coop: Vec::new(),
        };

        let state = OverlayState::from_history(&cache);
        let selected = state.selected.unwrap();
        assert_eq!(selected.id, "latest");
        assert_eq!(selected.power_delta, Some(20.0));
        assert_eq!(selected.kills, Some(9));
        assert_eq!(state.recent_summary.sample_size, 3);
        assert_eq!(state.recent_summary.win, 2);
        assert_eq!(state.recent_summary.lose, 1);
        assert!((state.recent_summary.win_rate - 200.0 / 3.0).abs() < 0.0001);
    }

    #[test]
    fn ignores_regular_battles_in_recent_summary_and_selection() {
        let cache = HistoryCache {
            schema_version: 1,
            fetched_at_ms: 1,
            battles: vec![battle(
                "regular",
                "2026-09-22T04:00:00Z",
                "REGULAR",
                None,
                "WIN",
            )],
            coop: Vec::new(),
        };

        let state = OverlayState::from_history(&cache);
        assert!(state.selected.is_none());
        assert_eq!(state.recent_summary.sample_size, 0);
    }

    #[test]
    fn keeps_missing_personal_result_as_missing_kad() {
        let mut record = battle(
            "no-result",
            "2026-09-22T04:00:00Z",
            "X_MATCH",
            Some(2019.8),
            "DEEMED_LOSE",
        );
        record.record.kills = None;
        record.record.assists = None;
        record.record.deaths = None;
        let state = OverlayState::from_history(&HistoryCache {
            schema_version: 1,
            fetched_at_ms: 1,
            battles: vec![record],
            coop: Vec::new(),
        });
        let selected = state.selected.unwrap();
        assert_eq!(selected.kills, None);
        assert_eq!(selected.assists, None);
        assert_eq!(selected.deaths, None);
    }

    #[test]
    fn x_power_delta_is_scoped_to_each_rule() {
        let mut clam_latest = battle(
            "clam-latest",
            "2026-09-22T03:00:00Z",
            "X_MATCH",
            Some(2020.0),
            "WIN",
        );
        clam_latest.raw["vsRule"] = serde_json::json!({"rule":"CLAM"});
        let mut tower = battle(
            "tower",
            "2026-09-22T02:00:00Z",
            "X_MATCH",
            Some(2100.0),
            "WIN",
        );
        tower.raw["vsRule"] = serde_json::json!({"rule":"LOFT"});
        let mut clam_previous = battle(
            "clam-previous",
            "2026-09-22T01:00:00Z",
            "X_MATCH",
            Some(2000.0),
            "WIN",
        );
        clam_previous.raw["vsRule"] = serde_json::json!({"rule":"CLAM"});

        let state = OverlayState::from_history(&HistoryCache {
            schema_version: 1,
            fetched_at_ms: 1,
            battles: vec![clam_latest, tower, clam_previous],
            coop: Vec::new(),
        });
        assert_eq!(state.selected.unwrap().power_delta, Some(20.0));
    }

    #[test]
    fn challenge_power_delta_is_scoped_to_weapon() {
        let mut weapon_a_latest = challenge_battle("weapon-a-latest", "03:00", 2010.0, "weapon-a");
        let weapon_b = challenge_battle("weapon-b", "02:00", 1900.0, "weapon-b");
        let weapon_a_previous = challenge_battle("weapon-a-old", "01:00", 2000.0, "weapon-a");
        weapon_a_latest.record.judgement = Some("WIN".to_owned());

        let state = OverlayState::from_history(&HistoryCache {
            schema_version: 1,
            fetched_at_ms: 1,
            battles: vec![weapon_a_latest, weapon_b, weapon_a_previous],
            coop: Vec::new(),
        });
        assert_eq!(state.selected.unwrap().power_delta, Some(10.0));
    }

    #[test]
    fn open_power_delta_is_scoped_to_team_independent_of_player_order() {
        let latest = open_battle("latest", "03:00", 1748.4, &["alpha", "beta"]);
        let other_team = open_battle("other-team", "02:00", 2000.0, &["alpha", "gamma"]);
        let previous = open_battle("previous", "01:00", 1700.0, &["beta", "alpha"]);

        let state = OverlayState::from_history(&HistoryCache {
            schema_version: 1,
            fetched_at_ms: 1,
            battles: vec![latest, other_team, previous],
            coop: Vec::new(),
        });
        assert!((state.selected.unwrap().power_delta.unwrap() - 48.4).abs() < 0.0001);
    }

    fn challenge_battle(id: &str, time: &str, power: f64, weapon_id: &str) -> ArchivedBattle {
        let mut item = battle(
            id,
            &format!("2026-09-22T{time}:00Z"),
            "BANKARA",
            Some(power),
            "WIN",
        );
        item.record.bankara_mode = Some("CHALLENGE".to_owned());
        item.record.power_kind = Some("weapon".to_owned());
        item.raw = serde_json::json!({
            "myTeam": { "players": [{ "isMyself": true, "weapon": { "id": weapon_id } }] }
        });
        item
    }

    fn open_battle(id: &str, time: &str, power: f64, player_ids: &[&str]) -> ArchivedBattle {
        let mut item = battle(
            id,
            &format!("2026-09-22T{time}:00Z"),
            "BANKARA",
            Some(power),
            "WIN",
        );
        item.record.bankara_mode = Some("OPEN".to_owned());
        item.record.power_kind = Some("team".to_owned());
        item.raw = serde_json::json!({
            "myTeam": { "players": player_ids.iter().map(|id| serde_json::json!({"id":id})).collect::<Vec<_>>() }
        });
        item
    }
}
