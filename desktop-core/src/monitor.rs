//! Current-only battle monitoring.
//!
//! The first observation establishes a baseline and emits nothing. Later
//! observations only emit battles newer than that baseline. This prevents a
//! newly added account from replaying old SplatNet history into the overlay.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BattleSummary {
    pub id: String,
    pub played_at_ms: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MonitorCursor {
    pub baseline_at_ms: i64,
    pub last_seen_battle_id: Option<String>,
    pub last_seen_at_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Observation {
    BaselineEstablished(MonitorCursor),
    NewBattles {
        battles: Vec<BattleSummary>,
        cursor: MonitorCursor,
    },
    NoChange(MonitorCursor),
}

#[derive(Debug, Default)]
pub struct CurrentBattleMonitor {
    cursor: Option<MonitorCursor>,
}

impl CurrentBattleMonitor {
    pub fn from_cursor(cursor: Option<MonitorCursor>) -> Self {
        Self { cursor }
    }

    pub fn cursor(&self) -> Option<&MonitorCursor> {
        self.cursor.as_ref()
    }

    /// Seed the monitor with the latest list without fetching any details.
    /// Existing entries are intentionally ignored.
    pub fn establish_baseline(
        &mut self,
        now_ms: i64,
        latest: impl IntoIterator<Item = BattleSummary>,
    ) -> Observation {
        let newest = latest.into_iter().max_by_key(|battle| battle.played_at_ms);
        let cursor = MonitorCursor {
            baseline_at_ms: now_ms,
            last_seen_battle_id: newest.as_ref().map(|battle| battle.id.clone()),
            last_seen_at_ms: newest.map(|battle| battle.played_at_ms),
        };
        self.cursor = Some(cursor.clone());
        Observation::BaselineEstablished(cursor)
    }

    /// Return only entries after the persisted cursor. The caller fetches
    /// details for these IDs; older entries never reach that layer.
    pub fn observe(
        &mut self,
        latest: impl IntoIterator<Item = BattleSummary>,
    ) -> Result<Observation, String> {
        let cursor = self
            .cursor
            .clone()
            .ok_or_else(|| "monitor baseline has not been established".to_owned())?;

        let mut battles: Vec<_> = latest
            .into_iter()
            .filter(|battle| is_newer_than_cursor(battle, &cursor))
            .collect();
        battles.sort_by_key(|battle| battle.played_at_ms);

        if battles.is_empty() {
            return Ok(Observation::NoChange(cursor));
        }

        let newest = battles
            .last()
            .expect("non-empty battle list after empty check");
        let next_cursor = MonitorCursor {
            baseline_at_ms: cursor.baseline_at_ms,
            last_seen_battle_id: Some(newest.id.clone()),
            last_seen_at_ms: Some(newest.played_at_ms),
        };
        self.cursor = Some(next_cursor.clone());
        Ok(Observation::NewBattles {
            battles,
            cursor: next_cursor,
        })
    }
}

fn is_newer_than_cursor(battle: &BattleSummary, cursor: &MonitorCursor) -> bool {
    if battle.played_at_ms < cursor.baseline_at_ms {
        return false;
    }

    match cursor.last_seen_at_ms {
        Some(last_seen_at) if battle.played_at_ms < last_seen_at => false,
        Some(last_seen_at) if battle.played_at_ms == last_seen_at => {
            cursor.last_seen_battle_id.as_deref() != Some(battle.id.as_str())
        }
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn battle(id: &str, time: i64) -> BattleSummary {
        BattleSummary {
            id: id.to_owned(),
            played_at_ms: time,
        }
    }

    #[test]
    fn first_observation_establishes_a_baseline_without_emitting_old_battles() {
        let mut monitor = CurrentBattleMonitor::default();
        let result = monitor.establish_baseline(200, vec![battle("old", 100)]);

        assert!(matches!(result, Observation::BaselineEstablished(_)));
        assert_eq!(
            monitor.cursor().unwrap().last_seen_battle_id.as_deref(),
            Some("old")
        );
    }

    #[test]
    fn only_new_battles_are_emitted_after_baseline() {
        let mut monitor = CurrentBattleMonitor::default();
        monitor.establish_baseline(200, vec![battle("old", 100)]);

        let result = monitor
            .observe(vec![battle("old", 100), battle("new", 300)])
            .unwrap();

        match result {
            Observation::NewBattles { battles, .. } => {
                assert_eq!(battles, vec![battle("new", 300)]);
            }
            other => panic!("expected new battle, got {other:?}"),
        }
    }

    #[test]
    fn cursor_prevents_duplicate_delivery_and_keeps_order() {
        let mut monitor = CurrentBattleMonitor::default();
        monitor.establish_baseline(100, vec![battle("old", 100)]);
        monitor
            .observe(vec![battle("one", 200), battle("two", 300)])
            .unwrap();

        let duplicate = monitor
            .observe(vec![battle("one", 200), battle("two", 300)])
            .unwrap();
        assert!(matches!(duplicate, Observation::NoChange(_)));

        let next = monitor
            .observe(vec![battle("two", 300), battle("three", 400)])
            .unwrap();
        match next {
            Observation::NewBattles { battles, .. } => {
                assert_eq!(battles, vec![battle("three", 400)]);
            }
            other => panic!("expected next battle, got {other:?}"),
        }
    }
}
