use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

const WEAPON_CATALOG_JSON: &str = include_str!("../resources/weapons.json");

static WEAPON_CATALOG: OnceLock<Result<Vec<Weapon>, String>> = OnceLock::new();

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct Weapon {
    pub id: i64,
    pub row_id: String,
    pub name: String,
    pub category_key: String,
    pub category: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(crate) struct PrivateWeaponSet {
    pub set_id: String,
    pub match_id: Option<String>,
    pub played_time: Option<String>,
    pub team_a: Vec<Weapon>,
    pub team_b: Vec<Weapon>,
}

pub(crate) fn catalog() -> Result<&'static [Weapon], String> {
    match WEAPON_CATALOG.get_or_init(|| {
        serde_json::from_str(WEAPON_CATALOG_JSON)
            .map_err(|error| format!("解析武器清单失败：{error}"))
    }) {
        Ok(weapons) => Ok(weapons),
        Err(error) => Err(error.clone()),
    }
}

pub(crate) fn create_set(
    match_id: Option<&str>,
    played_time: Option<String>,
) -> Result<PrivateWeaponSet, String> {
    let weapons = catalog()?;
    if weapons.is_empty() {
        return Err("武器清单为空".to_owned());
    }

    let mut random_values = [0_u8; 40];
    getrandom::fill(&mut random_values).map_err(|error| format!("生成随机武器失败：{error}"))?;
    let selected = random_values[..32]
        .chunks_exact(4)
        .map(|bytes| {
            let value = u32::from_ne_bytes(bytes.try_into().expect("four-byte random value"));
            weapons[(value as usize) % weapons.len()].clone()
        })
        .collect::<Vec<_>>();
    let set_id = random_values[32..]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();

    Ok(PrivateWeaponSet {
        set_id,
        match_id: match_id.map(str::to_owned),
        played_time,
        team_a: selected[..4].to_vec(),
        team_b: selected[4..].to_vec(),
    })
}

include!(concat!(env!("OUT_DIR"), "/weapon_assets.rs"));

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn catalog_contains_simplified_chinese_versus_weapons_and_local_images() {
        let weapons = catalog().unwrap();
        assert_eq!(weapons.len(), 131);
        assert!(weapons.iter().all(|weapon| !weapon.name.is_empty()));
        assert!(weapons.iter().any(|weapon| weapon.category == "射击枪"));
        assert!(weapons
            .iter()
            .all(|weapon| weapon_image(&weapon.row_id).is_some()));
    }

    #[test]
    fn each_private_match_assignment_has_eight_ordered_weapon_slots() {
        let set = create_set(
            Some("private-match-1"),
            Some("2026-09-23T00:00:00Z".to_owned()),
        )
        .unwrap();
        assert_eq!(set.match_id.as_deref(), Some("private-match-1"));
        assert_eq!(set.team_a.len(), 4);
        assert_eq!(set.team_b.len(), 4);
        let catalog_ids = catalog()
            .unwrap()
            .iter()
            .map(|weapon| weapon.id)
            .collect::<HashSet<_>>();
        assert!(set
            .team_a
            .iter()
            .chain(&set.team_b)
            .all(|weapon| catalog_ids.contains(&weapon.id)));
    }

    #[test]
    fn manual_set_can_be_created_without_a_private_battle() {
        let set = create_set(None, None).unwrap();
        assert_eq!(set.match_id, None);
        assert_eq!(set.played_time, None);
        assert_eq!(set.team_a.len(), 4);
        assert_eq!(set.team_b.len(), 4);
    }

    #[test]
    fn deserializes_existing_assignment_with_string_match_id() {
        let legacy = r#"{"set_id":"set","match_id":"private-match","played_time":null,"team_a":[],"team_b":[]}"#;
        let parsed: PrivateWeaponSet = serde_json::from_str(legacy).unwrap();
        assert_eq!(parsed.match_id.as_deref(), Some("private-match"));
    }
}
