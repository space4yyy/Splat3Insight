use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;

use serde::Deserialize;
use serde_json::json;
use splat3_widget_core::{
    history::HistoryCache,
    overlay::{OverlayState, RecentSummary},
};

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

#[derive(Debug, Deserialize)]
struct RegistrySnapshot {
    active_account_id: Option<String>,
}

pub fn spawn(config: OverlayServerConfig) {
    std::thread::spawn(move || {
        let address = format!("{OVERLAY_HOST}:{OVERLAY_PORT}");
        write_log(&config, &format!("开始监听 {address}"));
        let listener = match TcpListener::bind(&address) {
            Ok(listener) => listener,
            Err(error) => {
                write_log(&config, &format!("监听失败 {address}: {error}"));
                eprintln!("[overlay] 无法监听 {address}: {error}");
                return;
            }
        };

        write_log(&config, &format!("监听成功，OBS 地址: {OVERLAY_URL}"));
        eprintln!("[overlay] OBS 地址: {OVERLAY_URL}");
        for stream in listener.incoming() {
            match stream {
                Ok(stream) => {
                    let config = config.clone();
                    std::thread::spawn(move || {
                        if let Err(error) = handle_connection(stream, &config) {
                            eprintln!("[overlay] 请求失败: {error}");
                        }
                    });
                }
                Err(error) => eprintln!("[overlay] 接收请求失败: {error}"),
            }
        }
    });
}

fn write_log(config: &OverlayServerConfig, message: &str) {
    let Some(parent) = config.accounts_path.parent() else {
        return;
    };
    let path = parent.join("overlay.log");
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{message}");
    }
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

    if method != "GET" {
        return write_response(
            &mut stream,
            "405 Method Not Allowed",
            "text/plain; charset=utf-8",
            b"method not allowed",
        );
    }

    match path {
        "/" | "/overlay" => write_response(
            &mut stream,
            "200 OK",
            "text/html; charset=utf-8",
            OVERLAY_HTML.as_bytes(),
        ),
        "/fonts/DFP_GBZY7.ttf" => {
            write_response(&mut stream, "200 OK", "font/ttf", DISPLAY_FONT)
        }
        "/api/overlay" => match load_overlay_state(config) {
            Ok(state) => {
                let body = serde_json::to_vec(&state).map_err(|error| error.to_string())?;
                write_response(
                    &mut stream,
                    "200 OK",
                    "application/json; charset=utf-8",
                    &body,
                )
            }
            Err(error) => {
                let body = serde_json::to_vec(&json!({ "error": error }))
                    .map_err(|error| error.to_string())?;
                write_response(
                    &mut stream,
                    "200 OK",
                    "application/json; charset=utf-8",
                    &body,
                )
            }
        },
        "/healthz" => write_response(
            &mut stream,
            "200 OK",
            "application/json; charset=utf-8",
            br#"{"ok":true}"#,
        ),
        _ => write_response(
            &mut stream,
            "404 Not Found",
            "text/plain; charset=utf-8",
            b"not found",
        ),
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

    let history_path = config.history_dir.join(format!("{account_id}.json"));
    if !history_path.exists() {
        return Ok(empty_overlay_state());
    }
    let contents = fs::read_to_string(&history_path)
        .map_err(|error| format!("读取历史缓存失败: {error}"))?;
    let cache: HistoryCache =
        serde_json::from_str(&contents).map_err(|error| format!("历史缓存格式无效: {error}"))?;
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
