#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use tauri::{Manager, RunEvent};
use tracing_subscriber::EnvFilter;

use ytm_native::backend::{
    self, attach_close_to_tray, setup_system_tray, spawn_background_supervisor, AppController,
};

fn main() {
    ytm_native::updater::cleanup_old_binary();

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn"));
    let _ = tracing_subscriber::fmt().with_env_filter(filter).try_init();

    let app = tauri::Builder::default()
        .setup(|app| {
            let main_win = app.get_webview_window("main");
            let hwnd = main_win.as_ref().and_then(extract_hwnd);

            let (ctrl, media_rx) = AppController::new(hwnd)?;
            app.manage(ctrl.clone());

            if let Some(win) = main_win {
                attach_close_to_tray(&win);
            }

            setup_system_tray(app.handle(), ctrl.clone())?;
            spawn_background_supervisor(ctrl, media_rx);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            backend::get_playback_state,
            backend::get_library,
            backend::search_songs,
            backend::search_all,
            backend::get_artist,
            backend::toggle_follow_artist,
            backend::search_suggestions,
            backend::fetch_home_feed,
            backend::play_track,
            backend::play_collection,
            backend::select_queue_index,
            backend::prefetch_track,
            backend::toggle_pause,
            backend::play_next,
            backend::play_prev,
            backend::cycle_repeat,
            backend::toggle_shuffle,
            backend::set_sleep_timer,
            backend::get_lyrics,
            backend::save_integrations,
            backend::set_mini_player,
            backend::set_always_on_top,
            backend::save_custom_css,
            backend::mark_intro_seen,
            backend::set_language,
            backend::seek,
            backend::set_volume,
            backend::toggle_like,
            backend::create_playlist,
            backend::delete_playlist,
            backend::add_to_playlist,
            backend::remove_from_playlist,
            backend::connect_yt_browser,
            backend::connect_yt_cookies,
            backend::open_google_login,
            backend::sync_youtube_account,
            backend::start_mix,
            backend::start_artist_mix,
            backend::disconnect_yt,
            backend::check_update,
            backend::install_update,
        ])
        .build(tauri::generate_context!())
        .expect("error while building YTM Native");

    app.run(|_app_handle, event| {
        if let RunEvent::ExitRequested { api, code, .. } = event {
            // When the main window is destroyed to System Tray (code == None), keep the pure-Rust
            // audio engine alive in the background (~20-30 MB RAM)! Only exit on explicit Quit.
            if code.is_none() {
                api.prevent_exit();
            }
        }
    });
}

#[cfg(windows)]
fn extract_hwnd(win: &tauri::WebviewWindow) -> Option<*mut std::ffi::c_void> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    let handle = win.window_handle().ok()?;
    match handle.as_raw() {
        RawWindowHandle::Win32(w) => Some(w.hwnd.get() as *mut std::ffi::c_void),
        _ => None,
    }
}

#[cfg(not(windows))]
fn extract_hwnd(_win: &tauri::WebviewWindow) -> Option<*mut std::ffi::c_void> {
    None
}
