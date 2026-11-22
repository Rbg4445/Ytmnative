//! Background integrations for YTM Native 8:
//! - Discord Rich Presence (pure Rust IPC)
//! - Last.fm Now Playing & Scrobbling
//! - OBS Browser Source & Stream Deck Companion HTTP API (127.0.0.1:13091)
//! - System Tray icon & menu

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tracing::{info, warn};
use tray_icon::menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::player::Player;
use crate::rpc::{
    lastfm_scrobble, lastfm_update_now_playing, DiscordRpcClient, OBS_API_PORT, OBS_EMBED_HTML,
};

/// Spawn the Discord Rich Presence + Last.fm Scrobbler background task on the player's runtime.
pub fn spawn_integrations(player: Arc<Player>) {
    let p_obs = Arc::clone(&player);
    player.rt_handle().spawn(async move {
        run_obs_server(p_obs).await;
    });

    let p_rpc = Arc::clone(&player);
    player.rt_handle().spawn(async move {
        let mut discord = DiscordRpcClient::new();
        let mut lastfm_now_id = String::new();
        let mut lastfm_scrobbled_id = String::new();

        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;

            let snap = p_rpc.audio.snapshot();
            let current = p_rpc.queue.lock().ok().and_then(|q| q.current().cloned());
            let (
                discord_enabled,
                discord_client_id,
                lastfm_enabled,
                lastfm_key,
                lastfm_secret,
                lastfm_sk,
            ) = p_rpc
                .library
                .lock()
                .map(|l| {
                    (
                        l.discord_rpc_enabled,
                        l.discord_client_id.clone(),
                        l.lastfm_enabled,
                        l.lastfm_api_key.clone(),
                        l.lastfm_secret.clone(),
                        l.lastfm_session_key.clone(),
                    )
                })
                .unwrap_or((true, String::new(), false, String::new(), String::new(), String::new()));

            let is_playing = snap.is_playing && !snap.is_paused;
            let dur = if snap.duration_secs > 0.0 {
                snap.duration_secs
            } else {
                current.as_ref().and_then(|t| t.duration_secs).unwrap_or(0) as f64
            };

            if let Some(ref t) = current {
                discord.update_presence(
                    discord_enabled,
                    Some(&discord_client_id),
                    Some((
                        t.id.as_str(),
                        t.title.as_str(),
                        t.artist.as_str(),
                        t.thumbnail_url.as_deref(),
                    )),
                    is_playing,
                    snap.position_secs,
                    dur,
                );

                if lastfm_enabled && is_playing {
                    if lastfm_now_id != t.id {
                        lastfm_now_id = t.id.clone();
                        let http = p_rpc.http.clone();
                        let (k, s, sk, art, tit) = (
                            lastfm_key.clone(),
                            lastfm_secret.clone(),
                            lastfm_sk.clone(),
                            t.artist.clone(),
                            t.title.clone(),
                        );
                        tokio::spawn(async move {
                            lastfm_update_now_playing(&http, &k, &s, &sk, &art, &tit).await;
                        });
                    }

                    let threshold = if dur > 30.0 {
                        (dur * 0.5).min(240.0)
                    } else {
                        30.0
                    };
                    if dur >= 30.0 && snap.position_secs >= threshold && lastfm_scrobbled_id != t.id {
                        lastfm_scrobbled_id = t.id.clone();
                        let http = p_rpc.http.clone();
                        let ts = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .map(|d| d.as_secs())
                            .unwrap_or(0);
                        let (k, s, sk, art, tit) = (
                            lastfm_key.clone(),
                            lastfm_secret.clone(),
                            lastfm_sk.clone(),
                            t.artist.clone(),
                            t.title.clone(),
                        );
                        tokio::spawn(async move {
                            lastfm_scrobble(&http, &k, &s, &sk, &art, &tit, ts).await;
                        });
                    }
                }
            } else {
                discord.update_presence(false, None, None, false, 0.0, 0.0);
            }
        }
    });
}

async fn run_obs_server(player: Arc<Player>) {
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
        let player = Arc::clone(&player);
        tokio::spawn(async move {
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

            let enabled = player
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
                    let pb = player.snapshot_for_api();
                    let json = serde_json::json!({
                        "current": pb.current,
                        "is_playing": pb.is_playing && !pb.is_paused,
                        "is_paused": pb.is_paused,
                        "is_buffering": pb.is_buffering,
                        "position_secs": pb.position,
                        "duration_secs": pb.duration,
                        "volume": pb.volume,
                    })
                    .to_string();
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
                        player.next(false);
                    } else if raw_path.contains("prev") || req.contains("\"prev\"") {
                        player.prev();
                    } else if raw_path.contains("toggle")
                        || raw_path.contains("play")
                        || raw_path.contains("pause")
                        || req.contains("\"toggle\"")
                    {
                        player.toggle_pause();
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
}

// ============================================================================
// SYSTEM TRAY (Windows / Cross-platform via tray-icon)
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayAction {
    Show,
    TogglePlay,
    Prev,
    Next,
    Quit,
}

pub struct AppTray {
    _tray: TrayIcon,
    id_show: String,
    id_toggle: String,
    id_prev: String,
    id_next: String,
    id_quit: String,
}

impl AppTray {
    pub fn new(png_bytes: &[u8]) -> Option<Self> {
        let img = image_crate::load_from_memory(png_bytes).ok()?.to_rgba8();
        let (w, h) = img.dimensions();
        let icon = Icon::from_rgba(img.into_raw(), w, h).ok()?;

        let show = MenuItem::new("YTM Native'i Göster", true, None);
        let toggle = MenuItem::new("Oynat / Duraklat", true, None);
        let prev = MenuItem::new("Önceki Şarkı", true, None);
        let next = MenuItem::new("Sonraki Şarkı", true, None);
        let quit = MenuItem::new("Çıkış", true, None);

        let menu = Menu::new();
        let _ = menu.append(&show);
        let _ = menu.append(&PredefinedMenuItem::separator());
        let _ = menu.append(&toggle);
        let _ = menu.append(&prev);
        let _ = menu.append(&next);
        let _ = menu.append(&PredefinedMenuItem::separator());
        let _ = menu.append(&quit);

        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("YTM Native 8")
            .with_icon(icon)
            .build()
            .ok()?;

        Some(Self {
            _tray: tray,
            id_show: show.id().0.clone(),
            id_toggle: toggle.id().0.clone(),
            id_prev: prev.id().0.clone(),
            id_next: next.id().0.clone(),
            id_quit: quit.id().0.clone(),
        })
    }

    pub fn poll(&self) -> Vec<TrayAction> {
        let mut actions = Vec::new();
        while let Ok(ev) = TrayIconEvent::receiver().try_recv() {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = ev
            {
                actions.push(TrayAction::Show);
            }
        }
        while let Ok(ev) = MenuEvent::receiver().try_recv() {
            let id = ev.id.0;
            if id == self.id_show {
                actions.push(TrayAction::Show);
            } else if id == self.id_toggle {
                actions.push(TrayAction::TogglePlay);
            } else if id == self.id_prev {
                actions.push(TrayAction::Prev);
            } else if id == self.id_next {
                actions.push(TrayAction::Next);
            } else if id == self.id_quit {
                actions.push(TrayAction::Quit);
            }
        }
        actions
    }
}
