use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tracing::{info, warn};

use crate::backend::AppController;

pub use crate::lyrics::{fetch_lrclib_lyrics, parse_lrc, LyricLineDto};
pub use crate::rpc::{
    extract_video_id_from_url, lastfm_scrobble, lastfm_update_now_playing, md5_hex,
    DiscordRpcClient, DEFAULT_DISCORD_CLIENT_ID, OBS_API_PORT, OBS_EMBED_HTML,
};

pub fn spawn_obs_api_server(ctrl: Arc<AppController>) {
    tauri::async_runtime::spawn(async move {
        let addr = format!("127.0.0.1:{OBS_API_PORT}");
        let listener = match TcpListener::bind(&addr).await {
            Ok(l) => {
                info!("OBS Overlay & Companion API listening on http://{addr}");
                l
            }
            Err(e) => {
                warn!("Could not bind OBS API port {addr}: {e}");
                return;
            }
        };

        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                continue;
            };
            let ctrl = Arc::clone(&ctrl);
            tauri::async_runtime::spawn(async move {
                let mut buf = [0u8; 4096];
                let Ok(n) = stream.read(&mut buf).await else {
                    return;
                };
                if n == 0 {
                    return;
                }
                let req = String::from_utf8_lossy(&buf[..n]);
                let first_line = req.lines().next().unwrap_or("");
                let mut parts = first_line.split_whitespace();
                let method = parts.next().unwrap_or("GET");
                let raw_path = parts.next().unwrap_or("/");
                let path = raw_path.split('?').next().unwrap_or("/");

                let enabled = ctrl
                    .library
                    .lock()
                    .map(|l| l.obs_api_enabled)
                    .unwrap_or(true);

                if !enabled {
                    let resp = "HTTP/1.1 403 Forbidden\r\nContent-Type: text/plain\r\nContent-Length: 16\r\n\r\nOBS API Disabled";
                    let _ = stream.write_all(resp.as_bytes()).await;
                    return;
                }

                match (method, path) {
                    ("GET", "/") | ("GET", "/embed/now-playing") => {
                        let body = OBS_EMBED_HTML.as_bytes();
                        let hdr = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\n\r\n",
                            body.len()
                        );
                        let _ = stream.write_all(hdr.as_bytes()).await;
                        let _ = stream.write_all(body).await;
                    }
                    ("GET", "/api/v1/state") => {
                        let dto = ctrl.playback_dto();
                        let json = serde_json::to_string(&dto).unwrap_or_else(|_| "{}".to_string());
                        let body = json.as_bytes();
                        let hdr = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json; charset=utf-8\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\n\r\n",
                            body.len()
                        );
                        let _ = stream.write_all(hdr.as_bytes()).await;
                        let _ = stream.write_all(body).await;
                    }
                    ("POST", "/api/v1/command") | ("GET", "/api/v1/command") => {
                        if raw_path.contains("next") || req.contains("\"next\"") {
                            ctrl.advance_next(false);
                        } else if raw_path.contains("prev") || req.contains("\"prev\"") {
                            ctrl.step_prev();
                        } else if raw_path.contains("toggle") || raw_path.contains("play") || raw_path.contains("pause") || req.contains("\"toggle\"") {
                            ctrl.toggle_pause();
                        }
                        let body = b"{\"ok\":true}";
                        let hdr = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\n\r\n",
                            body.len()
                        );
                        let _ = stream.write_all(hdr.as_bytes()).await;
                        let _ = stream.write_all(body).await;
                    }
                    _ => {
                        let resp = "HTTP/1.1 404 Not Found\r\nContent-Length: 9\r\n\r\nNot Found";
                        let _ = stream.write_all(resp.as_bytes()).await;
                    }
                }
            });
        }
    });
}
