pub mod capture;
pub mod hotkeys;
pub mod math;
pub mod ring_buffer;
pub mod settings;
pub mod storage;
pub mod supervisor;

use std::sync::Arc;

use capture::{BufferStats, CaptureEngine, Monitor, WindowInfo};
use settings::Settings;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, State};

struct AppState {
    engine: Arc<CaptureEngine>,
}

#[tauri::command]
fn start_buffer(settings: Settings, state: State<AppState>, app: AppHandle) -> Result<(), String> {
    hotkeys::register_hotkeys(
        &app,
        &settings.hotkey_toggle_buffer,
        &settings.hotkey_save_continue,
        &settings.hotkey_save_stop,
        Arc::clone(&state.engine),
    );
    state.engine.start(settings).map_err(|e| e.to_string())
}

#[tauri::command]
fn stop_buffer(state: State<AppState>) -> Result<(), String> {
    state.engine.stop().map_err(|e| e.to_string())
}

#[tauri::command]
fn save_clip(continue_recording: bool, state: State<AppState>) -> Result<String, String> {
    state
        .engine
        .save_clip(continue_recording)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn get_buffer_stats(state: State<AppState>) -> BufferStats {
    state.engine.get_stats()
}

#[tauri::command]
fn get_settings() -> Settings {
    settings::load()
}

#[tauri::command]
fn save_settings(settings: Settings, state: State<AppState>, app: AppHandle) -> Result<(), String> {
    hotkeys::register_hotkeys(
        &app,
        &settings.hotkey_toggle_buffer,
        &settings.hotkey_save_continue,
        &settings.hotkey_save_stop,
        Arc::clone(&state.engine),
    );
    settings::save(&settings).map_err(|e| e.to_string())
}

#[tauri::command]
fn get_monitors() -> Vec<Monitor> {
    capture::enumerate_monitors()
}

#[tauri::command]
fn get_windows() -> Vec<WindowInfo> {
    capture::enumerate_windows()
}

#[tauri::command]
fn get_capacity_model(ram_limit_mb: u32, bitrate_mbps: f64) -> math::CapacityModel {
    math::CapacityModel::calculate(ram_limit_mb, bitrate_mbps)
}

#[tauri::command]
fn app_minimize(app: AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.minimize();
    }
}

#[tauri::command]
fn app_close(app: AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.close();
    }
}

#[tauri::command]
fn app_start_drag(app: AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        #[cfg(windows)]
        {
            if let Ok(hwnd) = w.hwnd() {
                #[link(name = "user32")]
                extern "system" {
                    fn ReleaseCapture() -> i32;
                    fn SendMessageW(hWnd: *mut std::ffi::c_void, Msg: u32, wParam: usize, lParam: isize) -> isize;
                }
                unsafe {
                    ReleaseCapture();
                    SendMessageW(hwnd.0 as _, 0x0112, 0xF012, 0);
                }
                return;
            }
        }
        let _ = w.start_dragging();
    }
}

#[cfg(windows)]
mod single_instance {
    type HANDLE = *mut std::ffi::c_void;
    type HWND = *mut std::ffi::c_void;
    type BOOL = i32;
    type DWORD = u32;
    type LPCWSTR = *const u16;

    const ERROR_ALREADY_EXISTS: DWORD = 183;
    const SW_RESTORE: i32 = 9;

    #[link(name = "kernel32")]
    extern "system" {
        fn CreateMutexW(lpMutexAttributes: *const std::ffi::c_void, bInitialOwner: BOOL, lpName: LPCWSTR) -> HANDLE;
        fn GetLastError() -> DWORD;
        fn CloseHandle(hObject: HANDLE) -> BOOL;
    }

    #[link(name = "user32")]
    extern "system" {
        fn FindWindowW(lpClassName: LPCWSTR, lpWindowName: LPCWSTR) -> HWND;
        fn ShowWindow(hWnd: HWND, nCmdShow: i32) -> BOOL;
        fn SetForegroundWindow(hWnd: HWND) -> BOOL;
    }

    pub fn acquire() -> Option<HANDLE> {
        let name: Vec<u16> = "Local\\dev.specter.single_instance.lock\0".encode_utf16().collect();
        let handle = unsafe { CreateMutexW(std::ptr::null(), 1, name.as_ptr()) };
        let last_err = unsafe { GetLastError() };

        if handle.is_null() || last_err == ERROR_ALREADY_EXISTS {
            unsafe {
                let win_name: Vec<u16> = "Specter\0".encode_utf16().collect();
                let hwnd = FindWindowW(std::ptr::null(), win_name.as_ptr());
                if !hwnd.is_null() {
                    ShowWindow(hwnd, SW_RESTORE);
                    SetForegroundWindow(hwnd);
                }
                if !handle.is_null() {
                    CloseHandle(handle);
                }
            }
            std::process::exit(0);
        }

        Some(handle)
    }
}

#[cfg(not(windows))]
mod single_instance {
    pub fn acquire() -> Option<()> {
        Some(())
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    #[cfg(windows)]
    let _instance_lock = single_instance::acquire();
    #[cfg(not(windows))]
    let _instance_lock = single_instance::acquire();

    let engine = Arc::new(CaptureEngine::new());

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .manage(AppState {
            engine: Arc::clone(&engine),
        })
        .setup({
            let engine = Arc::clone(&engine);
            move |app| {
                let loaded = settings::load();

                hotkeys::register_hotkeys(
                    app.handle(),
                    &loaded.hotkey_toggle_buffer,
                    &loaded.hotkey_save_continue,
                    &loaded.hotkey_save_stop,
                    Arc::clone(&engine),
                );

                let show_item = MenuItem::with_id(app, "show", "Open Specter", true, None::<&str>)?;
                let toggle_item = MenuItem::with_id(app, "toggle", "Toggle Buffering", true, None::<&str>)?;
                let save_item = MenuItem::with_id(app, "save", "Save Clip", true, None::<&str>)?;
                let quit_item = MenuItem::with_id(app, "quit", "Quit Specter", true, None::<&str>)?;
                let menu = Menu::with_items(app, &[&show_item, &toggle_item, &save_item, &quit_item])?;

                let engine_tray = Arc::clone(&engine);
                let _tray = TrayIconBuilder::with_id("specter-tray")
                    .tooltip("Specter - Instant Replay")
                    .icon(app.default_window_icon().unwrap().clone())
                    .menu(&menu)
                    .show_menu_on_left_click(false)
                    .on_menu_event(move |app, event| {
                        match event.id.as_ref() {
                            "show" => {
                                if let Some(w) = app.get_webview_window("main") {
                                    let _ = w.unminimize();
                                    let _ = w.show();
                                    let _ = w.set_focus();
                                }
                            }
                            "toggle" => {
                                if engine_tray.is_running() {
                                    let _ = engine_tray.stop();
                                    let _ = app.emit("buffering-status", false);
                                } else {
                                    let s = settings::load();
                                    if engine_tray.start(s).is_ok() {
                                        let _ = app.emit("buffering-status", true);
                                    }
                                }
                            }
                            "save" => {
                                if engine_tray.is_running() {
                                    let _ = engine_tray.save_clip(true);
                                }
                            }
                            "quit" => {
                                let _ = engine_tray.stop();
                                app.exit(0);
                            }
                            _ => {}
                        }
                    })
                    .on_tray_icon_event(|tray, event| {
                        match event {
                            TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. }
                            | TrayIconEvent::DoubleClick { button: MouseButton::Left, .. } => {
                                let app = tray.app_handle();
                                if let Some(w) = app.get_webview_window("main") {
                                    let is_minimized = w.is_minimized().unwrap_or(false);
                                    let is_visible = w.is_visible().unwrap_or(false);
                                    if is_visible && !is_minimized {
                                        let _ = w.set_focus();
                                    } else {
                                        let _ = w.unminimize();
                                        let _ = w.show();
                                        let _ = w.set_focus();
                                    }
                                }
                            }
                            _ => {}
                        }
                    })
                    .build(app)?;

                if let Some(w) = app.get_webview_window("main") {
                    if loaded.start_minimized {
                        let _ = w.hide();
                    } else {
                        let _ = w.show();
                        let _ = w.set_focus();
                    }
                }

                Ok(())
            }
        })
        .invoke_handler(tauri::generate_handler![
            start_buffer,
            stop_buffer,
            save_clip,
            get_buffer_stats,
            get_settings,
            save_settings,
            get_monitors,
            get_windows,
            get_capacity_model,
            app_minimize,
            app_close,
            app_start_drag
        ])
        .run(tauri::generate_context!())
        .expect("error while running Specter application");
}
