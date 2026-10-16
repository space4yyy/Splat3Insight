//! Data adapter for the compact B-style overlay card.

use serde::{Deserialize, Serialize};

use crate::history::{ArchivedBattle, BattleRecord, HistoryCache};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OverlayState {
    pub selected: Option<OverlayBattle>,
    pub recent_summary: RecentSummary,
    #[serde(default)]
    pub salmon_run: Option<SalmonRunCard>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SalmonRunCard {
    #[serde(default)]
    pub played_time: Option<String>,
    pub stage: Option<String>,
    #[serde(default)]
    pub species: Option<String>,
    #[serde(default)]
    pub boss_enemy_id: Option<i64>,
    #[serde(default)]
    pub boss_progress: Option<i64>,
    #[serde(default)]
    pub boss_encounter_count: Option<usize>,
    #[serde(default)]
    pub boss_undefeated_count: Option<usize>,
    pub gold_scale: Option<i64>,
    pub silver_scale: Option<i64>,
    pub bronze_scale: Option<i64>,
    pub defeat_enemy_count: Option<i64>,
    pub golden_egg_count: Option<i64>,
    pub power_egg_count: Option<i64>,
    pub rescued_count: Option<i64>,
    pub rescue_count: Option<i64>,
    pub clear_count: usize,
    pub failure_count: usize,
    pub grade: Option<String>,
    pub grade_point: Option<i64>,
    pub danger_rate: Option<f64>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayMatchCategory {
    XMatch,
    NonXMatch,
}

impl OverlayState {
    pub fn from_history(cache: &HistoryCache) -> Self {
        Self::from_history_filtered(cache, None)
    }

    pub fn from_history_for_match_category(
        cache: &HistoryCache,
        category: OverlayMatchCategory,
    ) -> Self {
        Self::from_history_filtered(cache, Some(category))
    }

    fn from_history_filtered(cache: &HistoryCache, category: Option<OverlayMatchCategory>) -> Self {
        let mut battles: Vec<&ArchivedBattle> = cache
            .battles
            .iter()
            .filter(|item| {
                is_overlay_battle(&item.record)
                    && match category {
                        Some(OverlayMatchCategory::XMatch) => item.record.mode == "X_MATCH",
                        Some(OverlayMatchCategory::NonXMatch) => item.record.mode != "X_MATCH",
                        None => true,
                    }
            })
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
            salmon_run: None,
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
    matches!(
        record.mode.as_str(),
        "X_MATCH" | "BANKARA" | "EVENT_MATCH" | "PRIVATE_BATTLE"
    )
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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
    EventMatch {
        power_kind: String,
        event_id: String,
        division: String,
        group_id: String,
    },
}

const CACHED_POWER_TRACK_KEY: &str = "__s3w_overlay_power_track_v1";

pub fn archived_battle_power_track_key(
    record: &BattleRecord,
    raw: &serde_json::Value,
) -> Option<String> {
    let item = ArchivedBattle {
        record: record.clone(),
        raw: raw.clone(),
    };
    power_track(&item).and_then(|track| serde_json::to_string(&track).ok())
}

fn power_track(item: &ArchivedBattle) -> Option<PowerTrack> {
    if let Some(serialized_track) = item
        .raw
        .get(CACHED_POWER_TRACK_KEY)
        .and_then(serde_json::Value::as_str)
    {
        return serde_json::from_str(serialized_track).ok();
    }

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
        ("EVENT_MATCH", _) => {
            let event_id = event_session_id(&item.raw)?;
            let (division, group_id) = event_group_identity(&item.raw)?;
            Some(PowerTrack::EventMatch {
                power_kind,
                event_id,
                division,
                group_id,
            })
        }
        _ => None,
    }
}

fn mode_level_delta_track(item: &ArchivedBattle) -> Option<&'static str> {
    let scope = match (
        item.record.mode.as_str(),
        item.record.bankara_mode.as_deref(),
    ) {
        ("BANKARA", Some("OPEN")) => "BANKARA_OPEN",
        ("EVENT_MATCH", _) => "EVENT_MATCH",
        _ => return None,
    };
    Some(scope)
}

fn same_power_delta_track(current: &ArchivedBattle, candidate: &ArchivedBattle) -> bool {
    match (
        mode_level_delta_track(current),
        mode_level_delta_track(candidate),
    ) {
        (Some(current_scope), Some(candidate_scope)) => current_scope == candidate_scope,
        (Some(_), None) | (None, Some(_)) => false,
        (None, None) => power_track(current)
            .zip(power_track(candidate))
            .is_some_and(|(current_track, candidate_track)| current_track == candidate_track),
    }
}

fn event_session_id(raw: &serde_json::Value) -> Option<String> {
    let event_id = identifier_at_any(
        raw,
        &[
            &["eventMatch", "eventId"],
            &["eventMatch", "id"],
            &["eventBattle", "eventId"],
            &["eventBattle", "id"],
            &["challenge", "eventId"],
            &["challenge", "id"],
            &["event", "eventId"],
            &["event", "id"],
            &["leagueMatch", "eventId"],
            &["leagueMatch", "id"],
            &["leagueMatch", "leagueMatchEvent", "id"],
            &["eventMatchId"],
            &["eventId"],
            &["eventBattleId"],
            &["challengeId"],
        ],
    )?;
    let time_slot = identifier_at_any(
        raw,
        &[
            &["eventMatch", "timeSlotId"],
            &["eventMatch", "scheduleId"],
            &["eventBattle", "timeSlotId"],
            &["eventBattle", "scheduleId"],
            &["leagueMatch", "timeSlotId"],
            &["leagueMatch", "scheduleId"],
            &["eventTimeSlotId"],
            &["timeSlotId"],
        ],
    );
    Some(match time_slot {
        Some(time_slot) => format!("{event_id}:{time_slot}"),
        None => event_id,
    })
}

fn event_group_identity(raw: &serde_json::Value) -> Option<(String, String)> {
    let explicit_type = string_at_any(
        raw,
        &[
            &["eventMatch", "participationType"],
            &["eventMatch", "groupType"],
            &["eventMatch", "division"],
            &["eventMatch", "category"],
            &["eventBattle", "participationType"],
            &["eventBattle", "groupType"],
            &["eventBattle", "division"],
            &["eventBattle", "category"],
            &["event", "participationType"],
            &["leagueMatch", "participationType"],
            &["leagueMatch", "groupType"],
            &["leagueMatch", "division"],
        ],
    )
    .map(|value| value.to_ascii_uppercase());
    let is_solo = bool_at_any(
        raw,
        &[
            &["eventMatch", "isSolo"],
            &["eventBattle", "isSolo"],
            &["event", "isSolo"],
            &["leagueMatch", "isSolo"],
        ],
    )
    .or_else(|| explicit_type.as_deref().map(|kind| kind == "SOLO"));

    if is_solo == Some(true) {
        return Some(("SOLO".to_owned(), "solo".to_owned()));
    }

    let group_id = identifier_at_any(
        raw,
        &[
            &["eventMatch", "groupId"],
            &["eventMatch", "partyId"],
            &["eventMatch", "teamId"],
            &["eventMatch", "group", "id"],
            &["eventMatch", "party", "id"],
            &["eventMatch", "team", "id"],
            &["eventBattle", "groupId"],
            &["eventBattle", "partyId"],
            &["eventBattle", "teamId"],
            &["eventBattle", "group", "id"],
            &["eventBattle", "party", "id"],
            &["eventBattle", "team", "id"],
            &["leagueMatch", "groupId"],
            &["leagueMatch", "partyId"],
            &["leagueMatch", "teamId"],
            &["leagueMatch", "group", "id"],
            &["leagueMatch", "party", "id"],
            &["leagueMatch", "team", "id"],
            &["eventGroupId"],
            &["partyId"],
        ],
    );
    let group_roster = group_roster_id(raw);
    let group_id = group_id.or(group_roster)?;
    let division = match explicit_type.as_deref() {
        Some("DUO") | Some("PAIR") => "DUO",
        Some("TEAM") | Some("GROUP") => "TEAM",
        _ => "GROUP",
    };
    Some((division.to_owned(), group_id))
}

fn group_roster_id(raw: &serde_json::Value) -> Option<String> {
    let paths: &[&[&str]] = &[
        &["eventMatch", "group", "members"],
        &["eventMatch", "group", "players"],
        &["eventMatch", "party", "members"],
        &["eventMatch", "party", "players"],
        &["eventMatch", "team", "members"],
        &["eventMatch", "team", "players"],
        &["eventBattle", "group", "members"],
        &["eventBattle", "group", "players"],
        &["eventBattle", "party", "members"],
        &["eventBattle", "party", "players"],
        &["eventBattle", "team", "members"],
        &["eventBattle", "team", "players"],
        &["leagueMatch", "group", "members"],
        &["leagueMatch", "group", "players"],
        &["leagueMatch", "party", "members"],
        &["leagueMatch", "party", "players"],
        &["leagueMatch", "team", "members"],
        &["leagueMatch", "team", "players"],
    ];
    let mut members = paths.iter().find_map(|path| {
        let mut value = raw;
        for key in *path {
            value = value.get(*key)?;
        }
        value.as_array().map(|members| {
            members
                .iter()
                .map(|member| identifier_at_any(member, &[&["id"], &["playerId"], &["nameId"]]))
                .collect::<Option<Vec<_>>>()
        })
    })??;
    if members.is_empty() {
        return None;
    }
    members.sort_unstable();
    Some(members.join(","))
}

fn string_at_any(value: &serde_json::Value, paths: &[&[&str]]) -> Option<String> {
    paths.iter().find_map(|path| {
        let mut current = value;
        for key in *path {
            current = current.get(*key)?;
        }
        current.as_str().map(ToOwned::to_owned)
    })
}

fn bool_at_any(value: &serde_json::Value, paths: &[&[&str]]) -> Option<bool> {
    paths.iter().find_map(|path| {
        let mut current = value;
        for key in *path {
            current = current.get(*key)?;
        }
        current.as_bool()
    })
}

fn identifier_at_any(value: &serde_json::Value, paths: &[&[&str]]) -> Option<String> {
    paths.iter().find_map(|path| {
        let mut current = value;
        for key in *path {
            current = current.get(*key)?;
        }
        current
            .as_str()
            .map(ToOwned::to_owned)
            .or_else(|| current.as_i64().map(|identifier| identifier.to_string()))
    })
}

fn event_power_is_placed(item: &ArchivedBattle, older: &[&ArchivedBattle]) -> bool {
    let Some(track @ PowerTrack::EventMatch { .. }) = power_track(item) else {
        // Without stable event and participation identifiers, the rating cannot be safely
        // separated from another event or a different group.
        return false;
    };
    1 + older
        .iter()
        .filter(|candidate| power_track(candidate).as_ref() == Some(&track))
        .count()
        >= 5
}

fn to_overlay_battle(item: &ArchivedBattle, older: &[&ArchivedBattle]) -> OverlayBattle {
    let current_track = power_track(item);
    let power = if item.record.mode == "PRIVATE_BATTLE"
        || (item.record.mode == "EVENT_MATCH" && !event_power_is_placed(item, older))
    {
        None
    } else {
        item.record.power
    };
    let power_delta = power.and_then(|current_power| {
        if mode_level_delta_track(item).is_none() && current_track.is_none() {
            return None;
        }
        Some(
            older
                .iter()
                .enumerate()
                .filter(|(index, candidate)| {
                    candidate.record.power.is_some()
                        && same_power_delta_track(item, candidate)
                        && (item.record.mode != "EVENT_MATCH"
                            || event_power_is_placed(candidate, &older[*index + 1..]))
                        && (candidate.record.power.unwrap() - current_power).abs() > 0.0001
                })
                .find_map(|(_, candidate)| {
                    candidate.record.power.map(|power| current_power - power)
                })
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
        power,
        power_kind: (item.record.mode != "PRIVATE_BATTLE")
            .then(|| item.record.power_kind.clone())
            .flatten(),
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
    fn open_power_delta_is_scoped_to_mode_instead_of_team() {
        let latest = open_battle("latest", "03:00", 1748.4, &["alpha", "beta"]);
        let mut other_team = open_battle("other-team", "02:00", 2000.0, &["gamma", "delta"]);
        other_team.record.power_kind = Some("different".to_owned());
        let previous = open_battle("previous", "01:00", 1700.0, &["beta", "alpha"]);

        let state = OverlayState::from_history(&HistoryCache {
            schema_version: 1,
            fetched_at_ms: 1,
            battles: vec![latest, other_team, previous],
            coop: Vec::new(),
        });
        assert!((state.selected.unwrap().power_delta.unwrap() + 251.6).abs() < 0.0001);
    }

    #[test]
    fn event_match_delta_is_scoped_to_mode_across_events_and_groups() {
        let mut battles = Vec::new();
        for index in 1..=5 {
            battles.push(event_battle(
                &format!("group-a-{index}"),
                &format!("2026-09-22T00:{index:02}:00Z"),
                Some(1800.0 + f64::from(index)),
                "event-a",
                "slot-1",
                false,
                Some("party-a"),
            ));
        }
        for index in 1..=5 {
            battles.push(event_battle(
                &format!("group-b-{index}"),
                &format!("2026-09-22T00:{:02}:00Z", index + 5),
                Some(2400.0 + f64::from(index)),
                "event-b",
                "slot-2",
                false,
                Some("party-b"),
            ));
        }
        for battle in battles
            .iter_mut()
            .filter(|battle| battle.record.id.starts_with("group-b-"))
        {
            battle.record.power_kind = Some("team-event".to_owned());
        }
        battles.push(event_battle(
            "group-a-6",
            "2026-09-22T00:11:00Z",
            Some(1815.0),
            "event-a",
            "slot-1",
            false,
            Some("party-a"),
        ));

        let state = OverlayState::from_history(&HistoryCache {
            schema_version: 1,
            fetched_at_ms: 1,
            battles,
            coop: Vec::new(),
        });
        let selected = state.selected.unwrap();
        assert_eq!(selected.id, "group-a-6");
        assert_eq!(selected.power, Some(1815.0));
        assert_eq!(selected.power_delta, Some(-590.0));
    }

    #[test]
    fn event_match_power_is_hidden_until_five_matches_are_placed() {
        let battles = (1..=4)
            .map(|index| {
                event_battle(
                    &format!("solo-{index}"),
                    &format!("2026-09-22T00:0{index}:00Z"),
                    Some(1800.0 + f64::from(index)),
                    "event-a",
                    "slot-1",
                    true,
                    None,
                )
            })
            .collect();
        let state = OverlayState::from_history(&HistoryCache {
            schema_version: 1,
            fetched_at_ms: 1,
            battles,
            coop: Vec::new(),
        });
        let selected = state.selected.unwrap();
        assert_eq!(selected.mode, "EVENT_MATCH");
        assert_eq!(selected.power, None);
        assert_eq!(selected.power_delta, None);
    }

    #[test]
    fn private_battle_keeps_only_combat_stats_without_power() {
        let mut item = battle(
            "private",
            "2026-09-22T04:00:00Z",
            "PRIVATE_BATTLE",
            Some(2400.0),
            "WIN",
        );
        item.record.power_kind = Some("x".to_owned());
        let state = OverlayState::from_history(&HistoryCache {
            schema_version: 1,
            fetched_at_ms: 1,
            battles: vec![item],
            coop: Vec::new(),
        });

        let selected = state.selected.unwrap();
        assert_eq!(selected.power, None);
        assert_eq!(selected.power_delta, None);
        assert_eq!(selected.kills, Some(9));
        assert_eq!(selected.deaths, Some(5));
        assert_eq!(selected.power_kind, None);
    }

    #[test]
    fn event_match_power_is_unavailable_without_stable_track_identity() {
        let item = battle(
            "event-without-track",
            "2026-09-22T04:00:00Z",
            "EVENT_MATCH",
            Some(1842.6),
            "WIN",
        );
        let state = OverlayState::from_history(&HistoryCache {
            schema_version: 1,
            fetched_at_ms: 1,
            battles: vec![item],
            coop: Vec::new(),
        });
        let selected = state.selected.unwrap();
        assert_eq!(selected.power, None);
        assert_eq!(selected.power_delta, None);
    }

    #[test]
    fn event_match_power_and_delta_are_scoped_to_event_slot_and_group() {
        let mut battles = Vec::new();
        for index in 1..=5 {
            battles.push(event_battle(
                &format!("solo-{index}"),
                &format!("2026-09-22T00:0{index}:00Z"),
                Some(1800.0 + f64::from(index)),
                "event-a",
                "slot-1",
                true,
                None,
            ));
        }
        battles.push(event_battle(
            "other-group",
            "2026-09-22T00:06:00Z",
            Some(2400.0),
            "event-a",
            "slot-1",
            false,
            Some("friends-a"),
        ));
        battles.push(event_battle(
            "other-slot",
            "2026-09-22T00:07:00Z",
            Some(2300.0),
            "event-a",
            "slot-2",
            true,
            None,
        ));
        battles.push(event_battle(
            "other-event",
            "2026-09-22T00:08:00Z",
            Some(2200.0),
            "event-b",
            "slot-1",
            true,
            None,
        ));
        battles.push(event_battle(
            "solo-6",
            "2026-09-22T00:09:00Z",
            Some(1815.0),
            "event-a",
            "slot-1",
            true,
            None,
        ));

        let state = OverlayState::from_history(&HistoryCache {
            schema_version: 1,
            fetched_at_ms: 1,
            battles,
            coop: Vec::new(),
        });
        let selected = state.selected.unwrap();
        assert_eq!(selected.id, "solo-6");
        assert_eq!(selected.power, Some(1815.0));
        assert_eq!(selected.power_delta, Some(10.0));
    }

    #[test]
    fn event_match_group_power_uses_group_identity_not_solo_rating() {
        let mut battles = Vec::new();
        for index in 1..=5 {
            battles.push(event_battle(
                &format!("group-a-{index}"),
                &format!("2026-09-22T00:0{index}:00Z"),
                Some(1700.0 + f64::from(index)),
                "event-a",
                "slot-1",
                false,
                Some("friends-a"),
            ));
        }
        battles.push(event_battle(
            "group-b",
            "2026-09-22T00:06:00Z",
            Some(2300.0),
            "event-a",
            "slot-1",
            false,
            Some("friends-b"),
        ));
        battles.push(event_battle(
            "group-a-6",
            "2026-09-22T00:07:00Z",
            Some(1715.0),
            "event-a",
            "slot-1",
            false,
            Some("friends-a"),
        ));

        let state = OverlayState::from_history(&HistoryCache {
            schema_version: 1,
            fetched_at_ms: 1,
            battles,
            coop: Vec::new(),
        });
        let selected = state.selected.unwrap();
        assert_eq!(selected.id, "group-a-6");
        assert_eq!(selected.power_delta, Some(10.0));
    }

    #[test]
    fn event_match_uses_league_match_event_and_team_ids() {
        let mut battles = (1..=5)
            .map(|index| {
                league_event_battle(
                    &format!("team-a-{index}"),
                    &format!("2026-09-22T00:0{index}:00Z"),
                    1700.0 + f64::from(index),
                    "event-a",
                    "team-a",
                )
            })
            .collect::<Vec<_>>();
        battles.push(league_event_battle(
            "team-b-1",
            "2026-09-22T00:06:00Z",
            2300.0,
            "event-a",
            "team-b",
        ));
        battles.push(league_event_battle(
            "team-a-6",
            "2026-09-22T00:07:00Z",
            1715.0,
            "event-a",
            "team-a",
        ));

        let state = OverlayState::from_history(&HistoryCache {
            schema_version: 1,
            fetched_at_ms: 1,
            battles,
            coop: Vec::new(),
        });
        let selected = state.selected.unwrap();
        assert_eq!(selected.id, "team-a-6");
        assert_eq!(selected.power_delta, Some(10.0));
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

    fn event_battle(
        id: &str,
        time: &str,
        power: Option<f64>,
        event_id: &str,
        time_slot_id: &str,
        is_solo: bool,
        group_id: Option<&str>,
    ) -> ArchivedBattle {
        let mut item = battle(id, time, "EVENT_MATCH", power, "WIN");
        item.record.power_kind = Some("event".to_owned());
        item.raw = serde_json::json!({
            "eventMatch": {
                "eventId": event_id,
                "timeSlotId": time_slot_id,
                "isSolo": is_solo,
                "groupId": group_id
            }
        });
        item
    }

    fn league_event_battle(
        id: &str,
        time: &str,
        power: f64,
        event_id: &str,
        team_id: &str,
    ) -> ArchivedBattle {
        let mut item = battle(id, time, "EVENT_MATCH", Some(power), "WIN");
        item.record.power_kind = Some("event".to_owned());
        item.raw = serde_json::json!({
            "leagueMatch": {
                "leagueMatchEvent": { "id": event_id },
                "myLeaguePower": power,
                "teamId": team_id
            }
        });
        item
    }
}
