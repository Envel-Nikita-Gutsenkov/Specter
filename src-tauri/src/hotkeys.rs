use tauri::{AppHandle, Emitter};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use crate::capture::CaptureEngine;
use std::sync::Arc;

pub fn register_hotkeys(
    app: &AppHandle,
    toggle_buffer_key: &str,
    save_continue_key: &str,
    save_stop_key: &str,
    engine: Arc<CaptureEngine>,
) {
    let _ = app.global_shortcut().unregister_all();

    if let Ok(shortcut) = toggle_buffer_key.parse::<Shortcut>() {
        let app_h = app.clone();
        let engine_toggle = Arc::clone(&engine);
        let _ = app.global_shortcut().on_shortcut(shortcut, move |_, _, event| {
            if event.state() == ShortcutState::Pressed {
                let is_running = engine_toggle.is_running();
                if is_running {
                    let _ = engine_toggle.stop();
                    let _ = app_h.emit("buffering-status", false);
                } else {
                    let s = crate::settings::load();
                    if engine_toggle.start(s).is_ok() {
                        let _ = app_h.emit("buffering-status", true);
                    }
                }
            }
        });
    }

    if let Ok(shortcut) = save_continue_key.parse::<Shortcut>() {
        let app_h = app.clone();
        let engine_c = Arc::clone(&engine);
        let _ = app.global_shortcut().on_shortcut(shortcut, move |_, _, event| {
            if event.state() == ShortcutState::Pressed {
                match engine_c.save_clip(true) {
                    Ok(path) => {
                        let _ = app_h.emit("clip-saved", path);
                    }
                    Err(e) => {
                        let _ = app_h.emit("clip-error", e.to_string());
                    }
                }
            }
        });
    }

    if let Ok(shortcut) = save_stop_key.parse::<Shortcut>() {
        let app_h = app.clone();
        let engine_s = Arc::clone(&engine);
        let _ = app.global_shortcut().on_shortcut(shortcut, move |_, _, event| {
            if event.state() == ShortcutState::Pressed {
                match engine_s.save_clip(false) {
                    Ok(path) => {
                        let _ = app_h.emit("clip-saved", path);
                    }
                    Err(e) => {
                        let _ = app_h.emit("clip-error", e.to_string());
                    }
                }
                let _ = app_h.emit("buffering-status", false);
            }
        });
    }
}

pub fn unregister_all(app: &AppHandle) {
    let _ = app.global_shortcut().unregister_all();
}
