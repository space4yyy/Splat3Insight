//! SplatNet 图片链接。
//!
//! 记录里的图片链接带 CloudFront 签名，大约三个月后失效，去掉签名直接访问会被拒绝。
//! 同一张图（同一把武器、同一张场地……）在不同记录里只有签名不同，所以按去掉签名后的
//! 路径识别同一张图，桌面端据此在本地缓存。只处理任天堂资源服务器上的图片。

use reqwest::{Client, Url};
use serde_json::Value;
use std::collections::HashSet;
use std::time::Duration;

const IMAGE_HOST: &str = "api.lp1.av5ja.srv.nintendo.net";
const IMAGE_PATH_PREFIX: &str = "/resources/prod/";
/// 单张图片的大小上限，超过就不缓存。
pub const MAX_IMAGE_BYTES: usize = 8 * 1024 * 1024;

/// 图片在缓存里的相对路径（去掉签名），例如 `v3/weapon_illust/abc_0.png`。
/// 不是任天堂资源服务器上的 https 图片，或路径里有可疑字符时返回 None。
pub fn image_key(url: &str) -> Option<String> {
    let parsed = Url::parse(url).ok()?;
    if parsed.scheme() != "https"
        || parsed.host_str() != Some(IMAGE_HOST)
        || parsed.port().is_some()
    {
        return None;
    }
    let key = parsed.path().strip_prefix(IMAGE_PATH_PREFIX)?;
    is_image_key(key).then(|| key.to_owned())
}

/// 缓存路径是否安全：至少两段，每段只含字母、数字和 `_-.`，不以点开头。
pub fn is_image_key(key: &str) -> bool {
    let segments: Vec<&str> = key.split('/').collect();
    segments.len() >= 2
        && segments.iter().all(|segment| {
            !segment.is_empty()
                && !segment.starts_with('.')
                && segment
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
        })
}

/// 链接的签名是否已经过期（`Expires` 是 Unix 秒）。没有 `Expires` 时按未过期处理。
pub fn image_link_expired(url: &str, now_secs: i64) -> bool {
    Url::parse(url)
        .ok()
        .and_then(|parsed| {
            parsed
                .query_pairs()
                .find(|(name, _)| name == "Expires")
                .and_then(|(_, value)| value.parse::<i64>().ok())
        })
        .is_some_and(|expires| expires <= now_secs)
}

/// 记录详情里会显示的图片，按记录类型列出字段路径；`*` 表示数组里的每一项。
const BATTLE_IMAGE_PATHS: &[&[&str]] = &[
    &["vsStage", "image", "url"],
    &["myTeam", "players", "*", "weapon", "image", "url"],
    &[
        "myTeam",
        "players",
        "*",
        "weapon",
        "subWeapon",
        "image",
        "url",
    ],
    &[
        "myTeam",
        "players",
        "*",
        "weapon",
        "specialWeapon",
        "image",
        "url",
    ],
    &[
        "myTeam",
        "players",
        "*",
        "nameplate",
        "background",
        "image",
        "url",
    ],
    &[
        "myTeam",
        "players",
        "*",
        "nameplate",
        "badges",
        "*",
        "image",
        "url",
    ],
    &["otherTeams", "*", "players", "*", "weapon", "image", "url"],
    &[
        "otherTeams",
        "*",
        "players",
        "*",
        "weapon",
        "subWeapon",
        "image",
        "url",
    ],
    &[
        "otherTeams",
        "*",
        "players",
        "*",
        "weapon",
        "specialWeapon",
        "image",
        "url",
    ],
    &[
        "otherTeams",
        "*",
        "players",
        "*",
        "nameplate",
        "background",
        "image",
        "url",
    ],
    &[
        "otherTeams",
        "*",
        "players",
        "*",
        "nameplate",
        "badges",
        "*",
        "image",
        "url",
    ],
    &["player", "headGear", "image", "url"],
    &["player", "headGear", "primaryGearPower", "image", "url"],
    &[
        "player",
        "headGear",
        "additionalGearPowers",
        "*",
        "image",
        "url",
    ],
    &["player", "clothingGear", "image", "url"],
    &["player", "clothingGear", "primaryGearPower", "image", "url"],
    &[
        "player",
        "clothingGear",
        "additionalGearPowers",
        "*",
        "image",
        "url",
    ],
    &["player", "shoesGear", "image", "url"],
    &["player", "shoesGear", "primaryGearPower", "image", "url"],
    &[
        "player",
        "shoesGear",
        "additionalGearPowers",
        "*",
        "image",
        "url",
    ],
];
const COOP_IMAGE_PATHS: &[&[&str]] = &[
    &["coopStage", "image", "url"],
    &["weapons", "*", "image", "url"],
    &["myResult", "weapons", "*", "image", "url"],
    &["myResult", "specialWeapon", "image", "url"],
    &[
        "myResult",
        "player",
        "nameplate",
        "background",
        "image",
        "url",
    ],
    &[
        "myResult",
        "player",
        "nameplate",
        "badges",
        "*",
        "image",
        "url",
    ],
    &["memberResults", "*", "weapons", "*", "image", "url"],
    &["memberResults", "*", "specialWeapon", "image", "url"],
    &[
        "memberResults",
        "*",
        "player",
        "nameplate",
        "background",
        "image",
        "url",
    ],
    &[
        "memberResults",
        "*",
        "player",
        "nameplate",
        "badges",
        "*",
        "image",
        "url",
    ],
    &["bossResult", "boss", "image", "url"],
    &["bossResults", "*", "boss", "image", "url"],
    &["enemyResults", "*", "enemy", "image", "url"],
    &["waveResults", "*", "specialWeapons", "*", "image", "url"],
];

fn collect<'a>(value: &'a Value, path: &[&str], out: &mut Vec<&'a str>) {
    let Some((first, rest)) = path.split_first() else {
        if let Some(url) = value.as_str() {
            out.push(url);
        }
        return;
    };
    if *first == "*" {
        for item in value.as_array().into_iter().flatten() {
            collect(item, rest, out);
        }
    } else if let Some(child) = value.get(*first) {
        collect(child, rest, out);
    }
}

/// 一条记录详情里会显示的图片链接，按图片去重（同一张图只留第一个链接）。
pub fn display_image_urls(record_type: &str, raw: &Value) -> Vec<String> {
    let paths = match record_type {
        "battle" => BATTLE_IMAGE_PATHS,
        "coop" => COOP_IMAGE_PATHS,
        _ => return Vec::new(),
    };
    let mut found = Vec::new();
    for path in paths {
        collect(raw, path, &mut found);
    }
    let mut seen = HashSet::new();
    found
        .into_iter()
        .filter(|url| image_key(url).is_some_and(|key| seen.insert(key)))
        .map(ToOwned::to_owned)
        .collect()
}

/// 下载图片用的客户端。图片链接自带签名，不需要也不会带上账号凭据。
#[derive(Clone)]
pub struct ImageClient {
    http: Client,
}

impl ImageClient {
    pub fn new() -> Result<Self, String> {
        Self::with_proxy(None)
    }

    pub fn with_proxy(proxy_address: Option<&str>) -> Result<Self, String> {
        crate::network::apply_proxy(
            Client::builder().timeout(Duration::from_secs(30)),
            proxy_address,
        )?
        .build()
        .map(|http| Self { http })
        .map_err(|error| format!("创建图片下载客户端失败：{error}"))
    }

    /// 下载一张图片，只接受任天堂资源服务器上的图片链接。
    pub async fn download(&self, url: &str) -> Result<Vec<u8>, ImageDownloadError> {
        if image_key(url).is_none() {
            return Err(ImageDownloadError::Rejected);
        }
        let response = self
            .http
            .get(url)
            .send()
            .await
            .map_err(|error| ImageDownloadError::Network(error.to_string()))?;
        let status = response.status();
        if !status.is_success() {
            return Err(ImageDownloadError::Status(status.as_u16()));
        }
        let is_image = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("image/"));
        if !is_image {
            return Err(ImageDownloadError::Rejected);
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|error| ImageDownloadError::Network(error.to_string()))?;
        if bytes.len() > MAX_IMAGE_BYTES {
            return Err(ImageDownloadError::Rejected);
        }
        Ok(bytes.to_vec())
    }
}

/// 图片下载失败的原因：网络问题可以稍后重试，其余（签名过期、不是图片）不必重试。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageDownloadError {
    Network(String),
    Status(u16),
    Rejected,
}

impl ImageDownloadError {
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Network(_) => true,
            Self::Status(status) => *status == 429 || *status >= 500,
            Self::Rejected => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const WEAPON: &str = "https://api.lp1.av5ja.srv.nintendo.net/resources/prod/v3/weapon_illust/abc123_0.png?Expires=1700000000&Signature=sig&Key-Pair-Id=K1";

    #[test]
    fn image_keys_drop_the_signature_and_reject_other_links() {
        assert_eq!(
            image_key(WEAPON).as_deref(),
            Some("v3/weapon_illust/abc123_0.png")
        );
        for url in [
            "http://api.lp1.av5ja.srv.nintendo.net/resources/prod/v3/a/b.png",
            "https://example.com/resources/prod/v3/a/b.png",
            "https://api.lp1.av5ja.srv.nintendo.net/api/graphql",
            // 路径里的 .. 会先被规范化，跳出资源目录的链接因此不再以资源路径开头。
            "https://api.lp1.av5ja.srv.nintendo.net/resources/prod/../../etc/passwd",
            "https://api.lp1.av5ja.srv.nintendo.net/resources/prod/b.png",
            "https://api.lp1.av5ja.srv.nintendo.net:8443/resources/prod/v3/a/b.png",
            "not a url",
        ] {
            assert_eq!(image_key(url), None, "{url}");
        }
    }

    #[test]
    fn expiry_is_read_from_the_signed_link() {
        assert!(image_link_expired(WEAPON, 1_700_000_000));
        assert!(!image_link_expired(WEAPON, 1_699_999_999));
        assert!(!image_link_expired(
            "https://api.lp1.av5ja.srv.nintendo.net/resources/prod/v3/a/b.png",
            i64::MAX
        ));
    }

    #[test]
    fn display_images_cover_players_gear_and_shifts_once_each() {
        let url = |name: &str| {
            format!("https://api.lp1.av5ja.srv.nintendo.net/resources/prod/v3/{name}.png?Expires=1")
        };
        let player = |weapon: &str| {
            json!({
                "weapon": {
                    "image": { "url": url(&format!("weapon_illust/{weapon}")) },
                    "subWeapon": { "image": { "url": url("sub_img/curling") } },
                    "specialWeapon": { "image": { "url": url("special_img/stamp") } }
                },
                "nameplate": {
                    "background": { "image": { "url": url("npl_img/plate") } },
                    "badges": [{ "image": { "url": url("badge_img/one") } }, null]
                }
            })
        };
        let battle = json!({
            "vsStage": { "image": { "url": url("stage_img/gorge") } },
            "myTeam": { "players": [player("shot"), player("roller")] },
            "otherTeams": [{ "players": [player("brush")] }],
            "player": { "headGear": {
                "image": { "url": url("gear_img/cap") },
                "primaryGearPower": { "image": { "url": url("skill_img/ink") } },
                "additionalGearPowers": [{ "image": { "url": url("skill_img/swim") } }]
            } },
            "awards": [{ "name": "#1 Turf Inker" }]
        });
        let keys: Vec<String> = display_image_urls("battle", &battle)
            .iter()
            .filter_map(|url| image_key(url))
            .collect();
        assert_eq!(keys.len(), 11);
        assert!(keys.contains(&"v3/stage_img/gorge.png".to_owned()));
        assert!(keys.contains(&"v3/weapon_illust/brush.png".to_owned()));
        assert!(keys.contains(&"v3/skill_img/swim.png".to_owned()));

        let coop = json!({
            "coopStage": { "image": { "url": url("stage_img/grounds") } },
            "weapons": [{ "image": { "url": url("weapon_illust/shot") } }],
            "myResult": { "weapons": [{ "image": { "url": url("weapon_illust/shot") } }] },
            "bossResult": { "boss": { "image": { "url": url("coop_enemy_img/king") } } },
            "enemyResults": [{ "enemy": { "image": { "url": url("coop_enemy_img/steelhead") } } }],
            "waveResults": [{ "specialWeapons": [{ "image": { "url": url("special_img/jet") } }] }]
        });
        assert_eq!(display_image_urls("coop", &coop).len(), 5);
        assert!(display_image_urls("meta", &coop).is_empty());
    }

    #[test]
    fn only_network_problems_are_retried() {
        assert!(ImageDownloadError::Network("timeout".to_owned()).is_retryable());
        assert!(ImageDownloadError::Status(503).is_retryable());
        assert!(!ImageDownloadError::Status(403).is_retryable());
        assert!(!ImageDownloadError::Rejected.is_retryable());
    }
}
