use std::fs;
use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::json;
use splat3_widget_core::overlay::{OverlayState, RecentSummary};

pub const OVERLAY_HOST: &str = "127.0.0.1";
pub const OVERLAY_PORT: u16 = 17_353;
pub const OVERLAY_URL: &str = "http://127.0.0.1:17353/overlay";

const OVERLAY_HTML: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../ui/overlay.html"));
const DISPLAY_FONT: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../ui/fonts/DFP_GBZY7.ttf"
));

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
    while !stop_signal.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, _)) => {
                if let Err(error) = stream.set_nonblocking(false) {
                    write_log(&config, &format!("设置客户端连接为阻塞模式失败: {error}"));
                    crate::log_console_error(
                        "overlay",
                        &format!("设置客户端连接为阻塞模式失败: {error}"),
                    );
                    continue;
                }
                set_status(&status, "running", "OBS 服务运行中", None);
                let config = config.clone();
                thread::spawn(move || {
                    if let Err(error) = handle_connection(stream, &config) {
                        write_log(&config, &format!("请求失败: {error}"));
                        crate::log_console_error("overlay", &format!("请求失败: {error}"));
                    }
                });
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

fn handle_connection(mut stream: TcpStream, config: &OverlayServerConfig) -> Result<(), String> {
    let mut buffer = [0_u8; 8 * 1024];
    let size = stream
        .read(&mut buffer)
        .map_err(|error| format!("读取请求失败: {error}"))?;
    let request = String::from_utf8_lossy(&buffer[..size]);
    let mut parts = request
        .lines()
        .next()
        .unwrap_or_default()
        .split_whitespace();
    let method = parts.next().unwrap_or_default();
    let raw_path = parts.next().unwrap_or_default();
    let path = raw_path.split('?').next().unwrap_or(raw_path);

    let response = route_request(method, path, config)?;
    write_response(
        &mut stream,
        response.status,
        response.content_type,
        &response.body,
    )
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
    let registry = fs::read_to_string(&config.accounts_path)
        .map_err(|error| format!("读取账号配置失败: {error}"))?;
    let snapshot: RegistrySnapshot =
        serde_json::from_str(&registry).map_err(|error| format!("读取账号配置失败: {error}"))?;
    let Some(account_id) = snapshot.active_account_id else {
        return Ok(empty_overlay_state());
    };
    if !is_safe_account_id(&account_id) {
        return Err("活动账号标识无效".to_owned());
    }

    let Some(cache) = crate::history_store::load_overlay_cache(&config.history_dir, &account_id)?
    else {
        return Ok(empty_overlay_state());
    };
    Ok(OverlayState::from_history(&cache))
}

fn empty_overlay_state() -> OverlayState {
    OverlayState {
        selected: None,
        recent_summary: RecentSummary {
            sample_size: 0,
            win: 0,
            lose: 0,
            win_rate: 0.0,
        },
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
    use tempfile::tempdir;

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
    fn overlay_api_returns_server_error_for_data_failures() {
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
}
