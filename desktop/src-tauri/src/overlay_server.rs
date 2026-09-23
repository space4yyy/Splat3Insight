use std::fs;
use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::json;
use splat3_widget_core::overlay::OverlayState;

pub const OVERLAY_HOST: &str = "127.0.0.1";
pub const OVERLAY_PORT: u16 = 17_353;
pub const OVERLAY_URL: &str = "http://127.0.0.1:17353/overlay";

const OVERLAY_HTML: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../ui/overlay.html"));
const OVERLAY_CARD_CSS: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../ui/overlay-card.css"
));
const OVERLAY_CARD_RENDERER: &str = include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../ui/overlay-card-renderer.js"
));
const DISPLAY_FONT: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../ui/fonts/DFP_GBZY7.ttf"
));
const MAX_CONCURRENT_CONNECTIONS: usize = 16;
const MAX_REQUEST_HEADER_BYTES: usize = 8 * 1024;
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
}

#[derive(Debug, Clone, Serialize)]
pub struct OverlayServiceStatus {
    pub state: String,
    pub url: String,
    pub message: String,
    pub error: Option<String>,
}

struct HttpResponse {
    status: &'static str,
    content_type: &'static str,
    body: Vec<u8>,
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
        url: OVERLAY_URL.to_owned(),
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
    write_log(&config, &format!("监听成功，OBS 地址: {OVERLAY_URL}"));
    crate::log_console("overlay", &format!("OBS 地址: {OVERLAY_URL}"));
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
    let path = raw_path.split('?').next().unwrap_or(raw_path);

    let response = route_request(method, path, config)?;
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
    path: &str,
    config: &OverlayServerConfig,
) -> Result<HttpResponse, String> {
    if method != "GET" {
        return Ok(HttpResponse {
            status: "405 Method Not Allowed",
            content_type: "text/plain; charset=utf-8",
            body: b"method not allowed".to_vec(),
        });
    }

    match path {
        "/" | "/overlay" => Ok(HttpResponse {
            status: "200 OK",
            content_type: "text/html; charset=utf-8",
            body: OVERLAY_HTML.as_bytes().to_vec(),
        }),
        "/fonts/DFP_GBZY7.ttf" => Ok(HttpResponse {
            status: "200 OK",
            content_type: "font/ttf",
            body: DISPLAY_FONT.to_vec(),
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
        "/api/overlay" => match load_overlay_state(config) {
            Ok(state) => Ok(HttpResponse {
                status: "200 OK",
                content_type: "application/json; charset=utf-8",
                body: serde_json::to_vec(&state).map_err(|error| error.to_string())?,
            }),
            Err(error) => {
                let message = format!("Overlay 数据请求失败: {error}");
                write_log(config, &message);
                crate::log_console_error("overlay", &message);
                Ok(HttpResponse {
                    status: "500 Internal Server Error",
                    content_type: "application/json; charset=utf-8",
                    body: serde_json::to_vec(&json!({ "error": error }))
                        .map_err(|error| error.to_string())?,
                })
            }
        },
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

fn load_overlay_state(config: &OverlayServerConfig) -> Result<OverlayState, String> {
    let registry = match fs::read_to_string(&config.accounts_path) {
        Ok(registry) => registry,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(crate::empty_overlay_state());
        }
        Err(error) => return Err(format!("读取账号配置失败: {error}")),
    };
    let snapshot: RegistrySnapshot =
        serde_json::from_str(&registry).map_err(|error| format!("读取账号配置失败: {error}"))?;
    let Some(account_id) = snapshot.active_account_id else {
        return Ok(crate::empty_overlay_state());
    };
    if !is_safe_account_id(&account_id) {
        return Err("活动账号标识无效".to_owned());
    }
    if snapshot
        .pending_account_deletions
        .iter()
        .any(|pending| pending == &account_id)
    {
        return Ok(crate::empty_overlay_state());
    }

    crate::history_store::load_overlay_state(&config.history_dir, &account_id)
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
            },
        )
        .unwrap();
        let body = String::from_utf8(response.body).unwrap();
        assert_eq!(response.status, "200 OK");
        assert!(body.contains("\"selected\":null"));
        assert!(body.contains("\"sample_size\":0"));
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
            },
        )
        .unwrap();
        assert_eq!(response.status, "200 OK");
        assert_eq!(response.content_type, "font/ttf");
        assert_eq!(response.body.len(), DISPLAY_FONT.len());
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
            },
        )
        .unwrap();

        assert_eq!(response.status, "200 OK");
        assert_eq!(response.content_type, "text/javascript; charset=utf-8");
        assert_eq!(response.body, OVERLAY_CARD_RENDERER.as_bytes());
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
