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

fn power_track(record: &BattleRecord) -> Option<(String, String)> {
    let power_kind = record.power_kind.as_ref()?.clone();
    Some((record.mode.clone(), power_kind))
}

fn to_overlay_battle(item: &ArchivedBattle, older: &[&ArchivedBattle]) -> OverlayBattle {
    let current_track = power_track(&item.record);
    let power_delta = item.record.power.and_then(|current_power| {
        older
            .iter()
            .filter(|candidate| {
                candidate.record.power.is_some()
                    && power_track(&candidate.record) == current_track
                    && (candidate.record.power.unwrap() - current_power).abs() > 0.0001
            })
            .find_map(|candidate| candidate.record.power.map(|power| current_power - power))
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
}
