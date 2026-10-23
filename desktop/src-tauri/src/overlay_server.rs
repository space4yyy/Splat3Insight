use std::fs;
use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::json;
use splat3_widget_core::dashboard::CoopDefeatFact;
use splat3_widget_core::overlay::{OverlayMatchCategory, OverlayState};

pub const OVERLAY_HOST: &str = "127.0.0.1";
pub const OVERLAY_PORT: u16 = 17_353;

const OBS_CARD_CATALOG: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../ui/obs-cards.json"));
const OVERLAY_HTML: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../ui/overlay.html"));
const CLOCK_HTML: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../ui/clock.html"));
const DEFEAT_COUNTS_HTML: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../ui/defeat-counts.html"
));
/// 击倒数量卡片的鲑鱼图片，后面接 coopEnemyId。
const COOP_ENEMY_IMAGE_ROUTE: &str = "/images/coop-enemy/";
const OVERLAY_CARD_CSS: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../ui/overlay-card.css"
));
const OVERLAY_CARD_RENDERER: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../ui/overlay-card-renderer.js"
));
const GAME_NAMES: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../ui/game-names.js"));
const DISPLAY_FONT: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../ui/fonts/DFP_GBZY7.ttf"
));
const BLITZ_MAIN_FONT: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../ui/fonts/BlitzMain.ttf"
));
const SCALE_GOLD: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../ui/images/scale-gold.png"
));
const SCALE_SILVER: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../ui/images/scale-silver.png"
));
const SCALE_BRONZE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../ui/images/scale-bronze.png"
));
const BOSS_1: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../ui/images/boss1.png"
));
const BOSS_2: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../ui/images/boss2.png"
));
const BOSS_3: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../ui/images/boss3.png"
));
const BOSS_1_COLOR: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../ui/images/boss1_color.png"
));
const BOSS_2_COLOR: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../ui/images/boss2_color.png"
));
const BOSS_3_COLOR: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../ui/images/boss3_color.png"
));
const BOSS_ALL: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../ui/images/boss_all.png"
));
const BOSS_ALL_COLOR: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../ui/images/boss_all_color.png"
));
const BOSS_WAVE: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../ui/images/wave.svg"
));
const IKURA: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../ui/images/ikura.svg"
));
const GOLD_IKURA: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../ui/images/gold-ikura.svg"
));
const HELPEN_SQUID: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../ui/images/helpen-squid.svg"
));
const HELPEN_OCTO: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../ui/images/helpen-octo.svg"
));
const HELPED_SQUID: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../ui/images/helped-squid.svg"
));
const HELPED_OCTO: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../ui/images/helped-octo.svg"
));
const APP_ICON: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/icons/icon.png"));
const MAX_CONCURRENT_CONNECTIONS: usize = 16;
const MAX_REQUEST_HEADER_BYTES: usize = 8 * 1024;

#[derive(Debug, Deserialize)]
struct ObsCardDefinition {
    id: String,
    route: String,
    page: String,
    width: u16,
    height: u16,
}

fn obs_card_catalog() -> &'static [ObsCardDefinition] {
    static CATALOG: OnceLock<Vec<ObsCardDefinition>> = OnceLock::new();
    CATALOG
        .get_or_init(|| {
            serde_json::from_str(OBS_CARD_CATALOG)
                .expect("embedded OBS card catalog must be valid JSON")
        })
        .as_slice()
}

fn overlay_url() -> String {
    let route = obs_card_catalog()
        .iter()
        .find(|card| card.id == "overlay")
        .map(|card| card.route.as_str())
        .expect("OBS card catalog must include the overlay card");
    format!("http://{OVERLAY_HOST}:{OVERLAY_PORT}{route}")
}

fn render_card_page(card: &ObsCardDefinition) -> HttpResponse {
    let template = match card.page.as_str() {
        "overlay" => OVERLAY_HTML,
        "clock" => CLOCK_HTML,
        "defeat-counts" => DEFEAT_COUNTS_HTML,
        _ => return not_found(),
    };
    let body = template
        .replace("__OBS_CARD_WIDTH__", &card.width.to_string())
        .replace("__OBS_CARD_HEIGHT__", &card.height.to_string());
    HttpResponse {
        status: "200 OK",
        content_type: "text/html; charset=utf-8",
        body: body.into_bytes(),
    }
}
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(5);
const OVERLOAD_WRITE_TIMEOUT: Duration = Duration::from_millis(250);

struct ConnectionPermit(Arc<AtomicUsize>);

impl Drop for ConnectionPermit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

fn try_reserve_connection(active: &AtomicUsize) -> bool {
    let mut current = active.load(Ordering::Acquire);
    loop {
        if current >= MAX_CONCURRENT_CONNECTIONS {
            return false;
        }
        match active.compare_exchange_weak(
            current,
            current + 1,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => return true,
            Err(actual) => current = actual,
        }
    }
}

#[derive(Clone)]
pub struct OverlayServerConfig {
    pub accounts_path: PathBuf,
    pub history_dir: PathBuf,
    /// Card data requests tell the background sync that OBS is showing the cards.
    pub(crate) activity: crate::sync_schedule::ObsActivity,
    /// The app's SplatNet image cache, for the Salmonid icons on the defeat count card.
    pub(crate) image_cache: Option<Arc<crate::image_cache::ImageCache>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OverlayServiceStatus {
    pub state: String,
    pub url: String,
    pub message: String,
    pub error: Option<String>,
}

pub(crate) struct HttpResponse {
    pub(crate) status: &'static str,
    pub(crate) content_type: &'static str,
    pub(crate) body: Vec<u8>,
}

pub type OverlayServiceStatusHandle = Arc<Mutex<OverlayServiceStatus>>;

#[derive(Clone)]
pub struct OverlayServiceController {
    inner: Arc<OverlayServiceControllerInner>,
}

struct OverlayServiceControllerInner {
    config: OverlayServerConfig,
    status: OverlayServiceStatusHandle,
    thread: Mutex<Option<JoinHandle<()>>>,
    stop_signal: Mutex<Option<Arc<AtomicBool>>>,
}

pub fn new_status() -> OverlayServiceStatusHandle {
    Arc::new(Mutex::new(OverlayServiceStatus {
        state: "stopped".to_owned(),
        url: overlay_url(),
        message: "OBS 服务未启动".to_owned(),
        error: None,
    }))
}

#[derive(Debug, Deserialize)]
struct RegistrySnapshot {
    active_account_id: Option<String>,
    #[serde(default)]
    pending_account_deletions: Vec<String>,
}

impl OverlayServiceController {
    pub fn new(config: OverlayServerConfig) -> Self {
        Self {
            inner: Arc::new(OverlayServiceControllerInner {
                config,
                status: new_status(),
                thread: Mutex::new(None),
                stop_signal: Mutex::new(None),
            }),
        }
    }

    pub fn status(&self) -> OverlayServiceStatusHandle {
        Arc::clone(&self.inner.status)
    }

    pub fn start(&self) -> Result<(), String> {
        let current_state = self
            .inner
            .status
            .lock()
            .map_err(|_| "OBS service status is locked")?
            .state
            .clone();
        if current_state == "running" || current_state == "starting" {
            return Ok(());
        }

        if let Some(thread) = self
            .inner
            .thread
            .lock()
            .map_err(|_| "OBS service thread is locked")?
            .take()
        {
            let _ = thread.join();
        }
        self.inner
            .stop_signal
            .lock()
            .map_err(|_| "OBS service stop signal is locked")?
            .take();

        let stop_signal = Arc::new(AtomicBool::new(false));
        *self
            .inner
            .stop_signal
            .lock()
            .map_err(|_| "OBS service stop signal is locked")? = Some(Arc::clone(&stop_signal));
        set_status(
            &self.inner.status,
            "starting",
            "正在启动 OBS 本地服务…",
            None,
        );

        let config = self.inner.config.clone();
        let status = Arc::clone(&self.inner.status);
        let thread = thread::spawn(move || run_server(config, status, stop_signal));
        *self
            .inner
            .thread
            .lock()
            .map_err(|_| "OBS service thread is locked")? = Some(thread);
        Ok(())
    }

    pub fn stop(&self) -> Result<(), String> {
        let current_state = self
            .inner
            .status
            .lock()
            .map_err(|_| "OBS service status is locked")?
            .state
            .clone();
        if current_state == "stopped" {
            return Ok(());
        }

        if let Some(stop_signal) = self
            .inner
            .stop_signal
            .lock()
            .map_err(|_| "OBS service stop signal is locked")?
            .take()
        {
            stop_signal.store(true, Ordering::SeqCst);
        }
        if let Some(thread) = self
            .inner
            .thread
            .lock()
            .map_err(|_| "OBS service thread is locked")?
            .take()
        {
            let _ = thread.join();
        }
        set_status(&self.inner.status, "stopped", "OBS 服务已停止", None);
        Ok(())
    }
}

fn run_server(
    config: OverlayServerConfig,
    status: OverlayServiceStatusHandle,
    stop_signal: Arc<AtomicBool>,
) {
    let address = format!("{OVERLAY_HOST}:{OVERLAY_PORT}");
    write_log(&config, &format!("开始监听 {address}"));
    let listener = match TcpListener::bind(&address) {
        Ok(listener) => listener,
        Err(error) => {
            let (message, detail) = if error.kind() == ErrorKind::AddrInUse {
                (
                    format!("OBS 服务启动失败：端口 {OVERLAY_PORT} 已被占用"),
                    format!(
                        "端口 {OVERLAY_PORT} 已被其他程序占用，请关闭占用程序后重新启动 OBS 服务"
                    ),
                )
            } else {
                (
                    format!("OBS 服务启动失败：无法监听 {address}"),
                    error.to_string(),
                )
            };
            set_status(&status, "error", &message, Some(detail));
            write_log(&config, &format!("监听失败 {address}: {error}"));
            crate::log_console_error("overlay", &format!("无法监听 {address}: {error}"));
            return;
        }
    };

    if let Err(error) = listener.set_nonblocking(true) {
        set_status(
            &status,
            "error",
            "OBS 服务启动失败：无法设置监听模式",
            Some(error.to_string()),
        );
        write_log(&config, &format!("设置非阻塞监听失败: {error}"));
        return;
    }

    set_status(&status, "running", "OBS 服务运行中", None);
    let url = overlay_url();
    write_log(&config, &format!("监听成功，OBS 地址: {url}"));
    crate::log_console("overlay", &format!("OBS 地址: {url}"));
    let active_connections = Arc::new(AtomicUsize::new(0));
    while !stop_signal.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((mut stream, _)) => {
                if let Err(error) = stream.set_nonblocking(false) {
                    write_log(&config, &format!("设置客户端连接为阻塞模式失败: {error}"));
                    crate::log_console_error(
                        "overlay",
                        &format!("设置客户端连接为阻塞模式失败: {error}"),
                    );
                    continue;
                }
                if !try_reserve_connection(&active_connections) {
                    let _ = stream.set_write_timeout(Some(OVERLOAD_WRITE_TIMEOUT));
                    let _ = write_response(
                        &mut stream,
                        "503 Service Unavailable",
                        "text/plain; charset=utf-8",
                        b"server busy",
                    );
                    continue;
                }
                set_status(&status, "running", "OBS 服务运行中", None);
                let worker_config = config.clone();
                let connection_permit = ConnectionPermit(Arc::clone(&active_connections));
                if let Err(error) = thread::Builder::new()
                    .name("overlay-http-client".to_owned())
                    .spawn(move || {
                        let _connection_permit = connection_permit;
                        if let Err(error) = handle_connection(stream, &worker_config) {
                            if !error.contains("请求读取超时") {
                                write_log(&worker_config, &format!("请求失败: {error}"));
                                crate::log_console_error("overlay", &format!("请求失败: {error}"));
                            }
                        }
                    })
                {
                    write_log(&config, &format!("创建 OBS 请求处理线程失败: {error}"));
                    crate::log_console_error(
                        "overlay",
                        &format!("创建 OBS 请求处理线程失败: {error}"),
                    );
                }
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(100));
            }
            Err(error) => {
                set_status(
                    &status,
                    "error",
                    "OBS 服务接收请求失败",
                    Some(error.to_string()),
                );
                write_log(&config, &format!("接收请求失败: {error}"));
                crate::log_console_error("overlay", &format!("接收请求失败: {error}"));
                break;
            }
        }
    }

    while active_connections.load(Ordering::Acquire) > 0 {
        thread::sleep(Duration::from_millis(20));
    }

    if stop_signal.load(Ordering::SeqCst) {
        write_log(&config, "OBS 服务已停止");
        set_status(&status, "stopped", "OBS 服务已停止", None);
    }
}

fn set_status(
    status: &OverlayServiceStatusHandle,
    state: &str,
    message: &str,
    error: Option<String>,
) {
    if let Ok(mut value) = status.lock() {
        value.state = state.to_owned();
        value.message = message.to_owned();
        value.error = error;
    }
}

fn write_log(config: &OverlayServerConfig, message: &str) {
    let Some(parent) = config.accounts_path.parent() else {
        return;
    };
    let path = parent.join("overlay.log");
    crate::append_timestamped_log(&path, "overlay", message);
}

fn handle_connection(stream: TcpStream, config: &OverlayServerConfig) -> Result<(), String> {
    handle_connection_with_timeout(stream, config, CONNECTION_TIMEOUT)
}

fn handle_connection_with_timeout(
    mut stream: TcpStream,
    config: &OverlayServerConfig,
    timeout: Duration,
) -> Result<(), String> {
    stream
        .set_read_timeout(Some(timeout))
        .map_err(|error| format!("设置请求读取超时失败: {error}"))?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(|error| format!("设置响应写入超时失败: {error}"))?;

    let Some(request_bytes) = read_request_headers(&mut stream)? else {
        let _ = write_response(
            &mut stream,
            "431 Request Header Fields Too Large",
            "text/plain; charset=utf-8",
            b"request headers too large",
        );
        return Ok(());
    };
    if request_bytes.is_empty() {
        return Ok(());
    }

    let request = String::from_utf8_lossy(&request_bytes);
    let mut parts = request
        .lines()
        .next()
        .unwrap_or_default()
        .split_whitespace();
    let method = parts.next().unwrap_or_default();
    let raw_path = parts.next().unwrap_or_default();
    let response = route_request(method, raw_path, config)?;
    if let Err(error) = write_response(
        &mut stream,
        response.status,
        response.content_type,
        &response.body,
    ) {
        // A browser may close the request while the response is being written.
        // This is a normal client disconnect and should not pollute the logs.
        if error.starts_with("写入响应失败:") {
            return Ok(());
        }
        return Err(error);
    }
    Ok(())
}

fn read_request_headers(reader: &mut impl Read) -> Result<Option<Vec<u8>>, String> {
    let mut request_bytes = Vec::with_capacity(2048);
    let mut chunk = [0_u8; 2048];
    while !request_bytes.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
        match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(size) => {
                request_bytes.extend_from_slice(&chunk[..size]);
                if request_bytes.len() > MAX_REQUEST_HEADER_BYTES {
                    return Ok(None);
                }
            }
            Err(error) if matches!(error.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock) => {
                return Err("请求读取超时".to_owned());
            }
            Err(error) => return Err(format!("读取请求失败: {error}")),
        }
    }
    Ok(Some(request_bytes))
}

fn route_request(
    method: &str,
    request_target: &str,
    config: &OverlayServerConfig,
) -> Result<HttpResponse, String> {
    if method != "GET" {
        return Ok(HttpResponse {
            status: "405 Method Not Allowed",
            content_type: "text/plain; charset=utf-8",
            body: b"method not allowed".to_vec(),
        });
    }

    let path = request_target.split('?').next().unwrap_or(request_target);

    if let Some(card) = obs_card_catalog()
        .iter()
        .find(|card| card.route == request_target)
    {
        return Ok(render_card_page(card));
    }

    if path == "/" {
        let card = obs_card_catalog()
            .iter()
            .find(|card| card.id == "overlay")
            .expect("OBS card catalog must include the overlay card");
        return Ok(render_card_page(card));
    }

    if let Some(card) = obs_card_catalog().iter().find(|card| card.route == path) {
        return Ok(render_card_page(card));
    }

    if let Some(enemy_id) = path.strip_prefix(COOP_ENEMY_IMAGE_ROUTE) {
        return Ok(enemy_id
            .parse()
            .ok()
            .and_then(|enemy_id| coop_enemy_image(config, enemy_id))
            .unwrap_or_else(not_found));
    }

    match path {
        "/favicon.png" => Ok(HttpResponse {
            status: "200 OK",
            content_type: "image/png",
            body: APP_ICON.to_vec(),
        }),
        "/obs-cards.json" => Ok(HttpResponse {
            status: "200 OK",
            content_type: "application/json; charset=utf-8",
            body: OBS_CARD_CATALOG.as_bytes().to_vec(),
        }),
        "/fonts/DFP_GBZY7.ttf" => Ok(HttpResponse {
            status: "200 OK",
            content_type: "font/ttf",
            body: DISPLAY_FONT.to_vec(),
        }),
        "/fonts/BlitzMain.ttf" => Ok(HttpResponse {
            status: "200 OK",
            content_type: "font/ttf",
            body: BLITZ_MAIN_FONT.to_vec(),
        }),
        "/images/scale-gold.png" => Ok(HttpResponse {
            status: "200 OK",
            content_type: "image/png",
            body: SCALE_GOLD.to_vec(),
        }),
        "/images/scale-silver.png" => Ok(HttpResponse {
            status: "200 OK",
            content_type: "image/png",
            body: SCALE_SILVER.to_vec(),
        }),
        "/images/scale-bronze.png" => Ok(HttpResponse {
            status: "200 OK",
            content_type: "image/png",
            body: SCALE_BRONZE.to_vec(),
        }),
        "/images/boss1.png" => Ok(HttpResponse {
            status: "200 OK",
            content_type: "image/png",
            body: BOSS_1.to_vec(),
        }),
        "/images/boss2.png" => Ok(HttpResponse {
            status: "200 OK",
            content_type: "image/png",
            body: BOSS_2.to_vec(),
        }),
        "/images/boss3.png" => Ok(HttpResponse {
            status: "200 OK",
            content_type: "image/png",
            body: BOSS_3.to_vec(),
        }),
        "/images/boss1_color.png" => Ok(HttpResponse {
            status: "200 OK",
            content_type: "image/png",
            body: BOSS_1_COLOR.to_vec(),
        }),
        "/images/boss2_color.png" => Ok(HttpResponse {
            status: "200 OK",
            content_type: "image/png",
            body: BOSS_2_COLOR.to_vec(),
        }),
        "/images/boss3_color.png" => Ok(HttpResponse {
            status: "200 OK",
            content_type: "image/png",
            body: BOSS_3_COLOR.to_vec(),
        }),
        "/images/boss_all.png" => Ok(HttpResponse {
            status: "200 OK",
            content_type: "image/png",
            body: BOSS_ALL.to_vec(),
        }),
        "/images/boss_all_color.png" => Ok(HttpResponse {
            status: "200 OK",
            content_type: "image/png",
            body: BOSS_ALL_COLOR.to_vec(),
        }),
        "/images/wave.svg" => Ok(HttpResponse {
            status: "200 OK",
            content_type: "image/svg+xml",
            body: BOSS_WAVE.to_vec(),
        }),
        "/images/ikura.svg" => Ok(HttpResponse {
            status: "200 OK",
            content_type: "image/svg+xml",
            body: IKURA.to_vec(),
        }),
        "/images/gold-ikura.svg" => Ok(HttpResponse {
            status: "200 OK",
            content_type: "image/svg+xml",
            body: GOLD_IKURA.to_vec(),
        }),
        "/images/helpen-squid.svg" => Ok(HttpResponse {
            status: "200 OK",
            content_type: "image/svg+xml",
            body: HELPEN_SQUID.to_vec(),
        }),
        "/images/helpen-octo.svg" => Ok(HttpResponse {
            status: "200 OK",
            content_type: "image/svg+xml",
            body: HELPEN_OCTO.to_vec(),
        }),
        "/images/helped-squid.svg" => Ok(HttpResponse {
            status: "200 OK",
            content_type: "image/svg+xml",
            body: HELPED_SQUID.to_vec(),
        }),
        "/images/helped-octo.svg" => Ok(HttpResponse {
            status: "200 OK",
            content_type: "image/svg+xml",
            body: HELPED_OCTO.to_vec(),
        }),
        "/overlay-card.css" => Ok(HttpResponse {
            status: "200 OK",
            content_type: "text/css; charset=utf-8",
            body: OVERLAY_CARD_CSS.as_bytes().to_vec(),
        }),
        "/overlay-card-renderer.js" => Ok(HttpResponse {
            status: "200 OK",
            content_type: "text/javascript; charset=utf-8",
            body: OVERLAY_CARD_RENDERER.as_bytes().to_vec(),
        }),
        "/game-names.js" => Ok(HttpResponse {
            status: "200 OK",
            content_type: "text/javascript; charset=utf-8",
            body: GAME_NAMES.as_bytes().to_vec(),
        }),
        "/api/overlay" => {
            config.activity.record_request(crate::now_ms());
            card_data_response(
                config,
                "Overlay 数据",
                load_overlay_state(config, overlay_match_category(request_target)),
            )
        }
        "/api/defeat-counts" => {
            config.activity.record_request(crate::now_ms());
            card_data_response(config, "击倒数量数据", load_defeat_counts(config))
        }
        "/healthz" => Ok(HttpResponse {
            status: "200 OK",
            content_type: "application/json; charset=utf-8",
            body: br#"{"ok":true}"#.to_vec(),
        }),
        _ => Ok(HttpResponse {
            status: "404 Not Found",
            content_type: "text/plain; charset=utf-8",
            body: b"not found".to_vec(),
        }),
    }
}

fn not_found() -> HttpResponse {
    HttpResponse {
        status: "404 Not Found",
        content_type: "text/plain; charset=utf-8",
        body: Vec::new(),
    }
}

fn write_response(
    stream: &mut TcpStream,
    status: &str,
    content_type: &str,
    body: &[u8],
) -> Result<(), String> {
    let header = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(header.as_bytes())
        .and_then(|_| stream.write_all(body))
        .map_err(|error| format!("写入响应失败: {error}"))
}

fn overlay_match_category(request_target: &str) -> Option<OverlayMatchCategory> {
    match request_target.split_once('?').map(|(_, query)| query) {
        Some("xmatch") => Some(OverlayMatchCategory::XMatch),
        Some("bankaramatch") => Some(OverlayMatchCategory::BankaraMatch),
        _ => None,
    }
}

/// 卡片数据接口的响应：成功时是数据本身，失败时记日志并返回 `{"error": …}`。
fn card_data_response<T: Serialize>(
    config: &OverlayServerConfig,
    label: &str,
    result: Result<T, String>,
) -> Result<HttpResponse, String> {
    match result {
        Ok(data) => Ok(HttpResponse {
            status: "200 OK",
            content_type: "application/json; charset=utf-8",
            body: serde_json::to_vec(&data).map_err(|error| error.to_string())?,
        }),
        Err(error) => {
            let message = format!("{label}请求失败: {error}");
            write_log(config, &message);
            crate::log_console_error("overlay", &message);
            Ok(HttpResponse {
                status: "500 Internal Server Error",
                content_type: "application/json; charset=utf-8",
                body: serde_json::to_vec(&json!({ "error": error }))
                    .map_err(|error| error.to_string())?,
            })
        }
    }
}

/// 卡片显示的账号：活动账号。没有账号配置、没有活动账号，或它正在被删除时为 None。
fn active_account(config: &OverlayServerConfig) -> Result<Option<String>, String> {
    let registry = match fs::read_to_string(&config.accounts_path) {
        Ok(registry) => registry,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("读取账号配置失败: {error}")),
    };
    let snapshot: RegistrySnapshot =
        serde_json::from_str(&registry).map_err(|error| format!("读取账号配置失败: {error}"))?;
    let Some(account_id) = snapshot.active_account_id else {
        return Ok(None);
    };
    if !is_safe_account_id(&account_id) {
        return Err("活动账号标识无效".to_owned());
    }
    if snapshot
        .pending_account_deletions
        .iter()
        .any(|pending| pending == &account_id)
    {
        return Ok(None);
    }
    Ok(Some(account_id))
}

/// 击倒数量卡片的数据：巨大鲑鱼（enemies）和头目鲑鱼（bosses），按 SplatNet 的顺序。
/// 图片换成本地服务的地址，OBS 不直接请求任天堂的链接。
#[derive(Debug, Serialize)]
struct DefeatCountCard {
    enemies: Vec<DefeatCountItem>,
    bosses: Vec<DefeatCountItem>,
}

#[derive(Debug, Serialize)]
struct DefeatCountItem {
    id: Option<i64>,
    name: Option<String>,
    image: Option<String>,
    count: i64,
}

/// 还没有活动账号，或还没查到「鲑鱼跑记录」时为 None，卡片隐藏。
fn load_defeat_counts(config: &OverlayServerConfig) -> Result<Option<DefeatCountCard>, String> {
    let Some(account_id) = active_account(config)? else {
        return Ok(None);
    };
    let Some(facts) = crate::dashboard_store::load_coop_defeats(&config.history_dir, &account_id)?
    else {
        return Ok(None);
    };
    let items = |list: Vec<CoopDefeatFact>| -> Vec<DefeatCountItem> {
        list.into_iter()
            .map(|fact| DefeatCountItem {
                image: fact
                    .image
                    .and(fact.enemy_id)
                    .map(|id| format!("{COOP_ENEMY_IMAGE_ROUTE}{id}")),
                id: fact.enemy_id,
                name: fact.name,
                count: fact.defeat_count,
            })
            .collect()
    };
    Ok(Some(DefeatCountCard {
        enemies: items(facts.enemies),
        bosses: items(facts.bosses),
    }))
}

/// 击倒数量卡片里的鲑鱼图片：按活动账号「鲑鱼跑记录」里的原链接读本地图片缓存，没有缓存时下载后存下。
/// 只给记录里有的鲑鱼，其他一律 404。
fn coop_enemy_image(config: &OverlayServerConfig, enemy_id: i64) -> Option<HttpResponse> {
    let cache = config.image_cache.as_ref()?;
    let account_id = active_account(config).ok()??;
    let facts =
        crate::dashboard_store::load_coop_defeats(&config.history_dir, &account_id).ok()??;
    let source = facts
        .enemies
        .into_iter()
        .chain(facts.bosses)
        .find(|fact| fact.enemy_id == Some(enemy_id))?
        .image?;
    let (content_type, body) =
        tauri::async_runtime::block_on(crate::image_cache::load(cache, &source))?;
    Some(HttpResponse {
        status: "200 OK",
        content_type,
        body,
    })
}

fn load_overlay_state(
    config: &OverlayServerConfig,
    category: Option<OverlayMatchCategory>,
) -> Result<OverlayState, String> {
    let Some(account_id) = active_account(config)? else {
        return Ok(crate::empty_overlay_state());
    };

    match category {
        Some(category) => crate::history_store::load_overlay_state_for_match_category(
            &config.history_dir,
            &account_id,
            category,
        ),
        None => crate::history_store::load_overlay_state(&config.history_dir, &account_id),
    }
}

fn is_safe_account_id(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

#[cfg(test)]
mod tests {
    use super::*;
    use splat3_widget_core::history::{ArchivedBattle, BattleRecord, HistoryCache};
    use std::collections::VecDeque;
    use std::io::{self, Cursor};
    use tempfile::tempdir;

    struct TestReader {
        chunks: VecDeque<io::Result<Vec<u8>>>,
        current: Cursor<Vec<u8>>,
    }

    impl TestReader {
        fn new(chunks: Vec<io::Result<Vec<u8>>>) -> Self {
            Self {
                chunks: chunks.into(),
                current: Cursor::new(Vec::new()),
            }
        }
    }

    impl Read for TestReader {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            if self.current.position() >= self.current.get_ref().len() as u64 {
                match self.chunks.pop_front() {
                    Some(Ok(chunk)) => self.current = Cursor::new(chunk),
                    Some(Err(error)) => return Err(error),
                    None => return Ok(0),
                }
            }
            self.current.read(buffer)
        }
    }

    fn sample_cache() -> HistoryCache {
        HistoryCache {
            schema_version: 1,
            fetched_at_ms: 1,
            battles: vec![ArchivedBattle {
                record: BattleRecord {
                    id: "battle-1".to_owned(),
                    played_time: Some("2026-09-22T00:00:00Z".to_owned()),
                    mode: "X_MATCH".to_owned(),
                    bankara_mode: None,
                    rule: Some("Clam Blitz".to_owned()),
                    stage: None,
                    judgement: Some("WIN".to_owned()),
                    player_name: None,
                    weapon: None,
                    paint: None,
                    power: Some(2000.0),
                    power_kind: Some("x".to_owned()),
                    kills: Some(8),
                    assists: Some(2),
                    deaths: Some(3),
                    specials: None,
                },
                raw: serde_json::json!({"id":"battle-1"}),
            }],
            coop: Vec::new(),
        }
    }

    #[test]
    fn overlay_api_returns_state_from_sqlite() {
        let directory = tempdir().unwrap();
        let accounts_path = directory.path().join("accounts.json");
        let history_dir = directory.path().join("history");
        std::fs::write(&accounts_path, r#"{"active_account_id":"account"}"#).unwrap();
        crate::history_store::save_cache(&history_dir, "account", &sample_cache()).unwrap();

        let response = route_request(
            "GET",
            "/api/overlay",
            &OverlayServerConfig {
                accounts_path,
                history_dir,
                activity: Default::default(),
                image_cache: None,
            },
        )
        .unwrap();
        assert_eq!(response.status, "200 OK");
        assert!(String::from_utf8(response.body)
            .unwrap()
            .contains("\"selected\""));
    }

    #[test]
    fn overlay_is_cleared_while_account_deletion_is_pending() {
        let directory = tempdir().unwrap();
        let accounts_path = directory.path().join("accounts.json");
        let history_dir = directory.path().join("history");
        crate::history_store::save_cache(&history_dir, "account", &sample_cache()).unwrap();
        std::fs::write(
            &accounts_path,
            r#"{"active_account_id":"account","pending_account_deletions":["account"]}"#,
        )
        .unwrap();

        let response = route_request(
            "GET",
            "/api/overlay",
            &OverlayServerConfig {
                accounts_path,
                history_dir,
                activity: Default::default(),
                image_cache: None,
            },
        )
        .unwrap();
        let body = String::from_utf8(response.body).unwrap();
        assert_eq!(response.status, "200 OK");
        assert!(body.contains("\"selected\":null"));
        assert!(body.contains("\"sample_size\":0"));
    }

    /// 「鲑鱼跑记录」：炸弹鱼、横纲带图片，头目联合没有图片也没击倒过。
    fn coop_record() -> serde_json::Value {
        let image = |name: &str| {
            format!("https://api.lp1.av5ja.srv.nintendo.net/resources/prod/v3/coop_enemy_img/{name}_0.png?Expires=1&Signature=s&Key-Pair-Id=k")
        };
        serde_json::json!({
            "defeatEnemyRecords": [{
                "enemy": { "coopEnemyId": 4, "name": "Steelhead", "image": { "url": image("steelhead") } },
                "defeatCount": 4812
            }],
            "defeatBossRecords": [
                {
                    "enemy": { "coopEnemyId": 23, "name": "Cohozuna", "image": { "url": image("cohozuna") } },
                    "defeatCount": 171
                },
                { "enemy": { "coopEnemyId": 30, "name": "Triumvirate" }, "defeatCount": 0 }
            ]
        })
    }

    fn active_account_config(
        directory: &std::path::Path,
        image_cache: Option<Arc<crate::image_cache::ImageCache>>,
    ) -> OverlayServerConfig {
        let accounts_path = directory.join("accounts.json");
        std::fs::write(&accounts_path, r#"{"active_account_id":"account"}"#).unwrap();
        OverlayServerConfig {
            accounts_path,
            history_dir: directory.join("history"),
            activity: Default::default(),
            image_cache,
        }
    }

    #[test]
    fn defeat_count_api_serves_the_coop_record_with_local_image_routes() {
        let directory = tempdir().unwrap();
        let config = active_account_config(directory.path(), None);
        // 还没查到「鲑鱼跑记录」：返回 null，卡片隐藏。
        let response = route_request("GET", "/api/defeat-counts", &config).unwrap();
        assert_eq!(response.status, "200 OK");
        assert_eq!(response.body, b"null");

        crate::dashboard_store::save_coop_record(&config.history_dir, "account", 1, &coop_record())
            .unwrap();
        let response = route_request("GET", "/api/defeat-counts", &config).unwrap();
        assert_eq!(response.status, "200 OK");
        assert_eq!(response.content_type, "application/json; charset=utf-8");
        let body: serde_json::Value = serde_json::from_slice(&response.body).unwrap();
        assert_eq!(
            body,
            json!({
                "enemies": [{ "id": 4, "name": "Steelhead", "image": "/images/coop-enemy/4", "count": 4812 }],
                "bosses": [
                    { "id": 23, "name": "Cohozuna", "image": "/images/coop-enemy/23", "count": 171 },
                    { "id": 30, "name": "Triumvirate", "image": null, "count": 0 }
                ]
            })
        );

        // 账号正在删除时不再显示。
        std::fs::write(
            &config.accounts_path,
            r#"{"active_account_id":"account","pending_account_deletions":["account"]}"#,
        )
        .unwrap();
        let response = route_request("GET", "/api/defeat-counts", &config).unwrap();
        assert_eq!(response.body, b"null");
    }

    #[test]
    fn coop_enemy_images_come_from_the_record_and_the_local_cache() {
        let directory = tempdir().unwrap();
        let cache_dir = directory.path().join("image-cache");
        crate::image_cache::store(
            &cache_dir,
            "v3/coop_enemy_img/steelhead_0.png",
            b"steelhead",
        )
        .unwrap();
        let config = active_account_config(
            directory.path(),
            Some(Arc::new(crate::image_cache::ImageCache {
                dir: cache_dir,
                client: std::sync::RwLock::new(
                    splat3_widget_core::images::ImageClient::new().unwrap(),
                ),
            })),
        );
        assert_eq!(
            route_request("GET", "/images/coop-enemy/4", &config)
                .unwrap()
                .status,
            "404 Not Found"
        );

        crate::dashboard_store::save_coop_record(&config.history_dir, "account", 1, &coop_record())
            .unwrap();
        let response = route_request("GET", "/images/coop-enemy/4", &config).unwrap();
        assert_eq!(
            (
                response.status,
                response.content_type,
                response.body.as_slice()
            ),
            ("200 OK", "image/png", &b"steelhead"[..])
        );
        // 记录里没有的鲑鱼、没有图片的头目鲑鱼和不是编号的地址都是 404，不会去下载。
        for path in [
            "/images/coop-enemy/99",
            "/images/coop-enemy/30",
            "/images/coop-enemy/4.png",
            "/images/coop-enemy/",
        ] {
            assert_eq!(
                route_request("GET", path, &config).unwrap().status,
                "404 Not Found",
                "{path}"
            );
        }
    }

    #[test]
    fn card_data_requests_tell_the_sync_that_obs_shows_the_cards() {
        let directory = tempdir().unwrap();
        for (page, requests) in [
            ("/overlay", ["/api/overlay", "/api/overlay?xmatch"]),
            (
                "/defeat-counts",
                ["/api/defeat-counts", "/api/defeat-counts"],
            ),
        ] {
            let starts = Arc::new(AtomicUsize::new(0));
            let counter = Arc::clone(&starts);
            let config = OverlayServerConfig {
                accounts_path: directory.path().join("missing-accounts.json"),
                history_dir: directory.path().join("history"),
                activity: crate::sync_schedule::ObsActivity::new(move || {
                    counter.fetch_add(1, Ordering::Relaxed);
                }),
                image_cache: None,
            };

            // Loading a card page is not enough; the card has to poll its data.
            route_request("GET", page, &config).unwrap();
            assert_eq!(starts.load(Ordering::Relaxed), 0, "{page}");
            assert!(!config.activity.is_live(crate::now_ms()));

            for request in requests {
                route_request("GET", request, &config).unwrap();
            }
            assert_eq!(starts.load(Ordering::Relaxed), 1, "{page}");
            assert!(config.activity.is_live(crate::now_ms()));
        }
    }

    #[test]
    fn overlay_api_returns_empty_state_when_account_registry_is_missing() {
        let directory = tempdir().unwrap();
        let response = route_request(
            "GET",
            "/api/overlay",
            &OverlayServerConfig {
                accounts_path: directory.path().join("missing-accounts.json"),
                history_dir: directory.path().join("history"),
                activity: Default::default(),
                image_cache: None,
            },
        )
        .unwrap();
        assert_eq!(response.status, "200 OK");
        let body = String::from_utf8(response.body).unwrap();
        assert!(body.contains("\"selected\":null"));
        assert!(body.contains("\"sample_size\":0"));
    }

    #[test]
    fn overlay_api_returns_server_error_for_invalid_account_registry() {
        let directory = tempdir().unwrap();
        let accounts_path = directory.path().join("accounts.json");
        std::fs::write(&accounts_path, "not-json").unwrap();
        let response = route_request(
            "GET",
            "/api/overlay",
            &OverlayServerConfig {
                accounts_path,
                history_dir: directory.path().join("history"),
                activity: Default::default(),
                image_cache: None,
            },
        )
        .unwrap();
        assert_eq!(response.status, "500 Internal Server Error");
        assert!(String::from_utf8(response.body)
            .unwrap()
            .contains("读取账号配置失败"));
    }

    #[test]
    fn serves_the_embedded_display_font() {
        let directory = tempdir().unwrap();
        let response = route_request(
            "GET",
            "/fonts/DFP_GBZY7.ttf",
            &OverlayServerConfig {
                accounts_path: directory.path().join("accounts.json"),
                history_dir: directory.path().join("history"),
                activity: Default::default(),
                image_cache: None,
            },
        )
        .unwrap();
        assert_eq!(response.status, "200 OK");
        assert_eq!(response.content_type, "font/ttf");
        assert_eq!(response.body.len(), DISPLAY_FONT.len());
    }

    #[test]
    fn serves_the_embedded_blitz_main_font() {
        let directory = tempdir().unwrap();
        let response = route_request(
            "GET",
            "/fonts/BlitzMain.ttf",
            &OverlayServerConfig {
                accounts_path: directory.path().join("accounts.json"),
                history_dir: directory.path().join("history"),
                activity: Default::default(),
                image_cache: None,
            },
        )
        .unwrap();
        assert_eq!(response.status, "200 OK");
        assert_eq!(response.content_type, "font/ttf");
        assert_eq!(response.body, BLITZ_MAIN_FONT);
        assert_eq!(&response.body[..4], &[0, 1, 0, 0]);
    }

    #[test]
    fn serves_the_app_icon_as_favicon() {
        let directory = tempdir().unwrap();
        let response = route_request(
            "GET",
            "/favicon.png",
            &OverlayServerConfig {
                accounts_path: directory.path().join("accounts.json"),
                history_dir: directory.path().join("history"),
                activity: Default::default(),
                image_cache: None,
            },
        )
        .unwrap();

        assert_eq!(response.status, "200 OK");
        assert_eq!(response.content_type, "image/png");
        assert_eq!(response.body, APP_ICON);
    }

    #[test]
    fn serves_the_shared_overlay_stylesheet() {
        let directory = tempdir().unwrap();
        let response = route_request(
            "GET",
            "/overlay-card.css",
            &OverlayServerConfig {
                accounts_path: directory.path().join("accounts.json"),
                history_dir: directory.path().join("history"),
                activity: Default::default(),
                image_cache: None,
            },
        )
        .unwrap();
        assert_eq!(response.status, "200 OK");
        assert_eq!(response.content_type, "text/css; charset=utf-8");
        assert!(String::from_utf8(response.body)
            .unwrap()
            .contains(".overlay-card__kad"));
    }

    #[test]
    fn serves_the_shared_overlay_card_renderer() {
        let directory = tempdir().unwrap();
        let response = route_request(
            "GET",
            "/overlay-card-renderer.js",
            &OverlayServerConfig {
                accounts_path: directory.path().join("accounts.json"),
                history_dir: directory.path().join("history"),
                activity: Default::default(),
                image_cache: None,
            },
        )
        .unwrap();

        assert_eq!(response.status, "200 OK");
        assert_eq!(response.content_type, "text/javascript; charset=utf-8");
        assert_eq!(response.body, OVERLAY_CARD_RENDERER.as_bytes());
    }

    #[test]
    fn serves_the_official_game_names_for_the_cards() {
        let directory = tempdir().unwrap();
        let response = route_request(
            "GET",
            "/game-names.js",
            &OverlayServerConfig {
                accounts_path: directory.path().join("accounts.json"),
                history_dir: directory.path().join("history"),
                activity: Default::default(),
                image_cache: None,
            },
        )
        .unwrap();

        assert_eq!(response.status, "200 OK");
        assert_eq!(response.content_type, "text/javascript; charset=utf-8");
        assert_eq!(response.body, GAME_NAMES.as_bytes());
        for page in [OVERLAY_HTML, DEFEAT_COUNTS_HTML] {
            let names_at = page.find("/game-names.js").unwrap();
            assert!(names_at < page.find("/overlay-card-renderer.js").unwrap());
        }
    }

    #[test]
    fn catalog_routes_render_pages_with_catalog_dimensions() {
        let directory = tempdir().unwrap();
        let config = OverlayServerConfig {
            accounts_path: directory.path().join("accounts.json"),
            history_dir: directory.path().join("history"),
            activity: Default::default(),
            image_cache: None,
        };

        for card in obs_card_catalog() {
            let response = route_request("GET", &card.route, &config).unwrap();
            let page = String::from_utf8(response.body).unwrap();
            assert_eq!(response.status, "200 OK", "{} route", card.id);
            assert!(
                page.contains(&format!("--obs-card-width: {}px", card.width)),
                "{} width is injected from catalog",
                card.id
            );
            assert!(
                page.contains(&format!("--obs-card-height: {}px", card.height)),
                "{} height is injected from catalog",
                card.id
            );
            assert!(!page.contains("__OBS_CARD_WIDTH__"));
            assert!(!page.contains("__OBS_CARD_HEIGHT__"));
        }
    }

    #[test]
    fn clock_card_keeps_local_time_without_polling_card_data() {
        let directory = tempdir().unwrap();
        let config = OverlayServerConfig {
            accounts_path: directory.path().join("accounts.json"),
            history_dir: directory.path().join("history"),
            activity: Default::default(),
            image_cache: None,
        };

        let response = route_request("GET", "/clock", &config).unwrap();
        assert_eq!(response.status, "200 OK");
        assert_eq!(response.content_type, "text/html; charset=utf-8");
        let page = String::from_utf8(response.body).unwrap();
        assert!(page.contains("/overlay-card.css"));
        assert!(page.contains("Splat3OverlayCard.updateClock(root)"));
        // 时钟页面不轮询卡片数据，后台同步不会因为它以为 OBS 正在显示数据卡片。
        assert!(!page.contains("/api/overlay"));
    }

    #[test]
    fn unavailable_routes_return_not_found() {
        let directory = tempdir().unwrap();
        let config = OverlayServerConfig {
            accounts_path: directory.path().join("accounts.json"),
            history_dir: directory.path().join("history"),
            activity: Default::default(),
            image_cache: None,
        };

        for path in [
            "/random-weapons",
            "/random-weapons-rules",
            "/api/random-weapons",
            "/weapons/Shooter_Short_00.png",
        ] {
            assert_eq!(
                route_request("GET", path, &config).unwrap().status,
                "404 Not Found"
            );
        }
        assert_eq!(
            route_request("POST", "/api/random-weapons/refresh", &config)
                .unwrap()
                .status,
            "405 Method Not Allowed"
        );
    }

    #[test]
    fn serves_the_same_card_catalog_used_by_routes() {
        let directory = tempdir().unwrap();
        let response = route_request(
            "GET",
            "/obs-cards.json",
            &OverlayServerConfig {
                accounts_path: directory.path().join("accounts.json"),
                history_dir: directory.path().join("history"),
                activity: Default::default(),
                image_cache: None,
            },
        )
        .unwrap();

        assert_eq!(response.status, "200 OK");
        assert_eq!(response.body, OBS_CARD_CATALOG.as_bytes());
        let parsed: Vec<ObsCardDefinition> = serde_json::from_slice(&response.body).unwrap();
        assert_eq!(parsed.len(), obs_card_catalog().len());
    }

    #[test]
    fn overlay_status_url_uses_catalog_route() {
        let status = new_status();
        assert_eq!(status.lock().unwrap().url, overlay_url());
        assert!(overlay_url().ends_with(
            &obs_card_catalog()
                .iter()
                .find(|card| card.id == "overlay")
                .unwrap()
                .route
        ));
    }

    #[test]
    fn connection_limit_is_enforced_and_released() {
        let active = AtomicUsize::new(MAX_CONCURRENT_CONNECTIONS);
        assert!(!try_reserve_connection(&active));
        active.fetch_sub(1, Ordering::AcqRel);
        assert!(try_reserve_connection(&active));
    }

    #[test]
    fn reads_fragmented_headers_and_handles_timeouts_and_oversized_headers() {
        let mut fragmented = TestReader::new(vec![
            Ok(b"GET /hea".to_vec()),
            Ok(b"lthz HTTP/1.1\r\nHost: localhost\r\n\r\n".to_vec()),
        ]);
        let request = read_request_headers(&mut fragmented).unwrap().unwrap();
        assert!(String::from_utf8(request)
            .unwrap()
            .starts_with("GET /healthz HTTP/1.1"));

        let mut timeout = TestReader::new(vec![Err(io::Error::new(
            ErrorKind::TimedOut,
            "test timeout",
        ))]);
        assert_eq!(
            read_request_headers(&mut timeout).unwrap_err(),
            "请求读取超时"
        );

        let mut oversized = TestReader::new(vec![Ok(vec![b'x'; MAX_REQUEST_HEADER_BYTES + 1])]);
        assert!(read_request_headers(&mut oversized).unwrap().is_none());
    }
}
