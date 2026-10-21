//! 本地图片缓存。
//!
//! SplatNet 图片链接带签名，大约三个月后失效。这里按图片路径（去掉签名）存一份，同一张图
//! 在不同记录里只存一次，旧记录的链接过期后仍能显示。
//! - 窗口用 `splat3img://localhost/<路径>?src=<原链接>` 读图：有缓存直接返回，没有时用原链接
//!   下载后存下来。
//! - 后台定期扫描新同步的记录，把详情里会显示的图片提前存好（签名已过期的跳过）。
//!
//! 缓存的是游戏素材（武器、场地、名牌等），不属于某个账号，删除账号时不清理。

use crate::history_store;
use rusqlite::params;
use splat3_widget_core::images::{
    display_image_urls, image_key, image_link_expired, is_image_key, ImageClient,
};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub(crate) const SCHEME: &str = "splat3img";
const MARKER_FILE: &str = ".scanned-rowid";
const SCAN_BATCH: i64 = 200;
const WARM_UP_INTERVAL: Duration = Duration::from_secs(60);
const DOWNLOAD_PAUSE: Duration = Duration::from_millis(80);

/// 托管在 Tauri 里的缓存位置和下载客户端。
pub(crate) struct ImageCache {
    pub(crate) dir: PathBuf,
    pub(crate) client: ImageClient,
}

fn cache_file(cache_dir: &Path, key: &str) -> PathBuf {
    cache_dir.join(key)
}

pub(crate) fn read_cached(cache_dir: &Path, key: &str) -> Option<Vec<u8>> {
    if !is_image_key(key) {
        return None;
    }
    fs::read(cache_file(cache_dir, key)).ok()
}

pub(crate) fn store(cache_dir: &Path, key: &str, bytes: &[u8]) -> Result<(), String> {
    if !is_image_key(key) {
        return Err(format!("图片缓存路径无效：{key}"));
    }
    let path = cache_file(cache_dir, key);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("创建图片缓存目录失败：{error}"))?;
    }
    let partial = path.with_file_name(format!(
        "{}.partial",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("image")
    ));
    fs::write(&partial, bytes).map_err(|error| format!("写入图片缓存失败：{error}"))?;
    fs::rename(&partial, &path).map_err(|error| {
        let _ = fs::remove_file(&partial);
        format!("保存图片缓存失败：{error}")
    })
}

pub(crate) fn content_type(key: &str) -> &'static str {
    match key
        .rsplit('.')
        .next()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("webp") => "image/webp",
        Some("gif") => "image/gif",
        _ => "application/octet-stream",
    }
}

/// 从请求的查询串里取出原链接 `src`。
fn source_link(query: Option<&str>) -> Option<String> {
    let query = query?;
    tauri::Url::parse(&format!("http://localhost/?{query}"))
        .ok()?
        .query_pairs()
        .find(|(name, _)| name == "src")
        .map(|(_, value)| value.into_owned())
}

/// 窗口里的一次读图请求：返回状态码和图片内容（没有图片时为空）。
/// 只在原链接指向同一张图时才下载，不会替页面去抓别的地址。
pub(crate) async fn respond(
    cache: &ImageCache,
    path: &str,
    query: Option<&str>,
) -> (u16, &'static str, Vec<u8>) {
    let key = path.trim_start_matches('/');
    if !is_image_key(key) {
        return (404, "text/plain", Vec::new());
    }
    if let Some(bytes) = read_cached(&cache.dir, key) {
        return (200, content_type(key), bytes);
    }
    let Some(source) = source_link(query) else {
        return (404, "text/plain", Vec::new());
    };
    if image_key(&source).as_deref() != Some(key) {
        return (404, "text/plain", Vec::new());
    }
    match load(cache, &source).await {
        Some((kind, bytes)) => (200, kind, bytes),
        None => (404, "text/plain", Vec::new()),
    }
}

/// 按原链接读图：有缓存直接返回，没有时下载后存下来。OBS 卡片的图片也走这里。
pub(crate) async fn load(cache: &ImageCache, source: &str) -> Option<(&'static str, Vec<u8>)> {
    let key = image_key(source)?;
    if let Some(bytes) = read_cached(&cache.dir, &key) {
        return Some((content_type(&key), bytes));
    }
    let bytes = cache.client.download(source).await.ok()?;
    if let Err(error) = store(&cache.dir, &key, &bytes) {
        crate::log_console_error("image-cache", &error);
    }
    Some((content_type(&key), bytes))
}

fn read_marker(cache_dir: &Path) -> i64 {
    fs::read_to_string(cache_dir.join(MARKER_FILE))
        .ok()
        .and_then(|text| text.trim().parse().ok())
        .unwrap_or(0)
}

fn write_marker(cache_dir: &Path, rowid: i64) -> Result<(), String> {
    fs::create_dir_all(cache_dir).map_err(|error| format!("创建图片缓存目录失败：{error}"))?;
    fs::write(cache_dir.join(MARKER_FILE), rowid.to_string())
        .map_err(|error| format!("保存图片缓存进度失败：{error}"))
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs() as i64)
        .unwrap_or(0)
}

/// 读取 rowid 大于 `after` 的一批记录，返回这批的最大 rowid，以及其中会显示、
/// 还没缓存、签名也没过期的图片链接（按图片去重）。没有新记录时返回 `after` 本身。
pub(crate) fn scan_batch(
    history_dir: &Path,
    cache_dir: &Path,
    after: i64,
    limit: i64,
    now: i64,
) -> Result<(i64, Vec<String>), String> {
    let connection = history_store::open_database(history_dir)?;
    // 数据库被重建时 rowid 会从头开始，进度也跟着从头算。
    let max_rowid: i64 = connection
        .query_row(
            "SELECT COALESCE(MAX(rowid), 0) FROM history_records",
            [],
            |row| row.get(0),
        )
        .map_err(|error| format!("读取图片缓存进度失败：{error}"))?;
    let after = if after > max_rowid { 0 } else { after };
    let mut statement = connection
        .prepare(
            "SELECT rowid, record_type, raw_json, raw_encoding FROM history_records
             WHERE rowid > ?1 AND record_type IN ('battle', 'coop')
             ORDER BY rowid LIMIT ?2",
        )
        .map_err(|error| format!("准备图片缓存扫描失败：{error}"))?;
    let rows = statement
        .query_map(params![after, limit], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Vec<u8>>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .map_err(|error| format!("读取图片缓存扫描失败：{error}"))?;
    let mut last = after;
    let mut seen = HashSet::new();
    let mut urls = Vec::new();
    for row in rows {
        let (rowid, record_type, raw_bytes, encoding) =
            row.map_err(|error| format!("读取图片缓存扫描行失败：{error}"))?;
        last = rowid;
        let Ok(raw) = history_store::decode_raw_json(&raw_bytes, &encoding) else {
            continue;
        };
        for url in display_image_urls(&record_type, &raw) {
            let Some(key) = image_key(&url) else {
                continue;
            };
            if image_link_expired(&url, now)
                || !seen.insert(key.clone())
                || cache_file(cache_dir, &key).exists()
            {
                continue;
            }
            urls.push(url);
        }
    }
    Ok((last, urls))
}

/// 把还没扫描过的记录里要显示的图片存好；遇到网络问题就停下，下次从同一批重试。
async fn warm_up_once(history_dir: &Path, cache: &ImageCache) -> Result<(), String> {
    loop {
        let after = read_marker(&cache.dir);
        let (history_dir_owned, cache_dir_owned) = (history_dir.to_path_buf(), cache.dir.clone());
        let (last, urls) = tauri::async_runtime::spawn_blocking(move || {
            scan_batch(
                &history_dir_owned,
                &cache_dir_owned,
                after,
                SCAN_BATCH,
                now_secs(),
            )
        })
        .await
        .map_err(|error| format!("扫描图片缓存失败：{error}"))??;
        if last == after {
            return Ok(());
        }
        for url in urls {
            let Some(key) = image_key(&url) else {
                continue;
            };
            if cache_file(&cache.dir, &key).exists() {
                continue;
            }
            match cache.client.download(&url).await {
                Ok(bytes) => store(&cache.dir, &key, &bytes)?,
                Err(error) if error.is_retryable() => return Ok(()),
                // 签名过期或不是图片：跳过，以后浏览时能用新链接的话再存。
                Err(_) => {}
            }
            tokio::time::sleep(DOWNLOAD_PAUSE).await;
        }
        write_marker(&cache.dir, last)?;
    }
}

/// 后台循环：启动后先补存现有记录里的图片，之后每分钟看一次有没有新同步的记录。
pub(crate) async fn run_warm_up(history_dir: PathBuf, cache: std::sync::Arc<ImageCache>) {
    loop {
        if let Err(error) = warm_up_once(&history_dir, &cache).await {
            crate::log_console_error("image-cache", &error);
        }
        tokio::time::sleep(WARM_UP_INTERVAL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history_store::{save_cache, tests_support::sample_cache_with_coop};
    use serde_json::json;
    use tempfile::tempdir;

    #[test]
    fn stores_and_reads_images_by_safe_key_only() {
        let directory = tempdir().unwrap();
        store(directory.path(), "v3/weapon_illust/shot_0.png", b"png").unwrap();
        assert_eq!(
            read_cached(directory.path(), "v3/weapon_illust/shot_0.png").as_deref(),
            Some(&b"png"[..])
        );
        assert!(store(directory.path(), "../escape.png", b"x").is_err());
        assert_eq!(read_cached(directory.path(), "v3/../../etc/passwd"), None);
        assert_eq!(content_type("v3/stage_img/a.PNG"), "image/png");
        assert_eq!(
            content_type("v3/stage_img/a.bin"),
            "application/octet-stream"
        );
    }

    #[test]
    fn requests_serve_cached_images_and_refuse_mismatched_sources() {
        let directory = tempdir().unwrap();
        let cache = ImageCache {
            dir: directory.path().to_path_buf(),
            client: ImageClient::new().unwrap(),
        };
        store(directory.path(), "v3/stage_img/gorge.png", b"stage").unwrap();
        let (status, kind, body) =
            tauri::async_runtime::block_on(respond(&cache, "/v3/stage_img/gorge.png", None));
        assert_eq!(
            (status, kind, body.as_slice()),
            (200, "image/png", &b"stage"[..])
        );
        // 原链接指向另一张图：不下载。
        let other = "src=https%3A%2F%2Fapi.lp1.av5ja.srv.nintendo.net%2Fresources%2Fprod%2Fv3%2Fstage_img%2Fother.png";
        let (status, _, _) = tauri::async_runtime::block_on(respond(
            &cache,
            "/v3/stage_img/missing.png",
            Some(other),
        ));
        assert_eq!(status, 404);
        let foreign =
            "src=https%3A%2F%2Fexample.com%2Fresources%2Fprod%2Fv3%2Fstage_img%2Fmissing.png";
        let (status, _, _) = tauri::async_runtime::block_on(respond(
            &cache,
            "/v3/stage_img/missing.png",
            Some(foreign),
        ));
        assert_eq!(status, 404);
        let (status, _, _) =
            tauri::async_runtime::block_on(respond(&cache, "/../secret.png", None));
        assert_eq!(status, 404);
    }

    #[test]
    fn scanning_lists_new_uncached_unexpired_images_once() {
        let history = tempdir().unwrap();
        let images = tempdir().unwrap();
        let url = |name: &str, expires: i64| {
            format!("https://api.lp1.av5ja.srv.nintendo.net/resources/prod/v3/{name}.png?Expires={expires}&Signature=s")
        };
        let mut cache = sample_cache_with_coop();
        cache.battles[0].raw = json!({
            "vsStage": { "image": { "url": url("stage_img/gorge", 2_000) } },
            "myTeam": { "players": [
                { "weapon": { "image": { "url": url("weapon_illust/shot", 2_000) } } },
                { "weapon": { "image": { "url": url("weapon_illust/old", 500) } } },
                { "weapon": { "image": { "url": url("weapon_illust/cached", 2_000) } } }
            ] }
        });
        cache.coop[0].raw = json!({
            "coopStage": { "image": { "url": url("stage_img/gorge", 2_000) } },
            "weapons": [{ "image": { "url": url("weapon_illust/shot", 2_000) } }]
        });
        save_cache(history.path(), "account", &cache).unwrap();
        store(images.path(), "v3/weapon_illust/cached.png", b"x").unwrap();

        let (last, urls) = scan_batch(history.path(), images.path(), 0, 50, 1_000).unwrap();
        let keys: Vec<String> = urls.iter().filter_map(|url| image_key(url)).collect();
        assert_eq!(
            keys,
            ["v3/stage_img/gorge.png", "v3/weapon_illust/shot.png"]
        );
        assert!(last > 0);
        // 进度之后没有新记录。
        let (again, more) = scan_batch(history.path(), images.path(), last, 50, 1_000).unwrap();
        assert_eq!((again, more.len()), (last, 0));
        // 进度比数据库还新（数据库被重建）时从头扫描。
        let (restart, _) =
            scan_batch(history.path(), images.path(), last + 100, 50, 1_000).unwrap();
        assert_eq!(restart, last);
    }
}
