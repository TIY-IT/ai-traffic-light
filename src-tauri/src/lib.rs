mod hooks;
mod bridge;
mod marvis;
mod presence;
mod zcode;
mod coze;
mod trae;
mod doubao;
mod codex_local;
mod workbuddy_local;
mod reporter;
mod notify;
mod focus;
mod sessions;

use sessions::{clear_sessions, delete_session, StatusSnapshot};
use std::fs;
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::Duration;
use tauri::{
    image::Image,
    menu::{CheckMenuItemBuilder, MenuBuilder, MenuItemBuilder},
    tray::TrayIconBuilder,
    AppHandle, Manager,
};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};

// Frontend-published clickable rects (logical px, window-relative). Anything
// outside them must pass clicks through, because the window is transparent
// and would otherwise swallow hits meant for windows underneath.
static HIT_RECTS: OnceLock<Mutex<Vec<[f64; 4]>>> = OnceLock::new();

fn hit_rects() -> &'static Mutex<Vec<[f64; 4]>> {
    HIT_RECTS.get_or_init(|| Mutex::new(Vec::new()))
}

// rects are logical [x, y, w, h]; cursor is window-relative physical px.
fn rects_contain(
    rects: &[[f64; 4]],
    scale: f64,
    cursor_window_relative_px: (f64, f64),
    margin_px: f64,
) -> bool {
    let (cx, cy) = cursor_window_relative_px;
    rects.iter().any(|r| {
        let (x, y, w, h) = (r[0] * scale, r[1] * scale, r[2] * scale, r[3] * scale);
        cx >= x - margin_px && cx <= x + w + margin_px && cy >= y - margin_px && cy <= y + h + margin_px
    })
}

#[tauri::command]
fn set_hit_regions(_window: tauri::WebviewWindow, rects: Vec<[f64; 4]>) {
    *hit_rects().lock().unwrap() = rects;
}

// 系统通知开关只存在后端(托盘菜单改写);前端提示音播放前用它对齐同一开关。
#[tauri::command]
fn notify_enabled() -> bool {
    notify::enabled()
}

// 点击面板里的会话行时,把该会话所属的 AI 客户端窗口调到前台。
#[tauri::command]
fn focus_session(client: String, id: String) -> Result<(), String> {
    focus::focus_session(&client, &id)
}

// The user drags the panel to their preferred spot; remember it across restarts.
fn window_pos_file(app: &AppHandle) -> Option<std::path::PathBuf> {
    app.path().resolve(".ai-traffic-light/window-pos.json", tauri::path::BaseDirectory::Home).ok()
}

fn restore_window_position(app: &AppHandle) {
    let Some(window) = app.get_webview_window("main") else { return };
    let Some(file) = window_pos_file(app) else { return };
    if let Ok(text) = fs::read_to_string(&file) {
        let nums: Vec<f64> = text
            .split(',')
            .filter_map(|v| v.trim().parse().ok())
            .collect();
        if nums.len() == 2 {
            let (mut x, mut y) = (nums[0], nums[1]);
            // A saved position dragged mostly off-screen must not make the window
            // vanish on startup: clamp so at least 80×80 physical px stay on a monitor.
            if let (Ok(size), Ok(monitors)) = (window.outer_size(), window.available_monitors()) {
                let (w, h) = (size.width as f64, size.height as f64);
                let mon = monitors
                    .iter()
                    .find(|m| {
                        let (mp, ms) = (m.position(), m.size());
                        x >= mp.x as f64 && x < (mp.x + ms.width as i32) as f64
                            && y >= mp.y as f64 && y < (mp.y + ms.height as i32) as f64
                    })
                    .or_else(|| monitors.first());
                if let Some(mon) = mon {
                    let (mp, ms) = (mon.position(), mon.size());
                    let (mx, my, mw, mh) =
                        (mp.x as f64, mp.y as f64, ms.width as f64, ms.height as f64);
                    // f64::clamp panics when min > max (window larger than monitor).
                    let (min_x, max_x) = (mx + 80.0 - w, mx + mw - 80.0);
                    let (min_y, max_y) = (my + 80.0 - h, my + mh - 80.0);
                    x = if min_x <= max_x { x.clamp(min_x, max_x) } else { mx };
                    y = if min_y <= max_y { y.clamp(min_y, max_y) } else { my };
                }
            }
            let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
        }
    }
}

fn save_window_position(app: &AppHandle, x: f64, y: f64) {
    if let Some(file) = window_pos_file(app) {
        let _ = fs::write(&file, format!("{},{}", x as i32, y as i32));
    }
}

// The window width follows the visible lamp count. Called once per frontend render
// (max ~2Hz): a backend resize loop fired continuously, made the WebView relayout
// nonstop, and could crash it.
#[tauri::command]
fn resize_light_to_fit(window: tauri::WebviewWindow, width: f64) {
    let want = width.clamp(180.0, 700.0);
    if let Ok(size) = window.outer_size() {
        let scale = window.scale_factor().unwrap_or(1.0);
        let cur = size.width as f64 / scale;
        if (want - cur).abs() > 4.0 {
            // Top-left stays fixed; the width grows rightward only. Never call
            // set_position here — the window must not move by itself.
            let _ = window.set_size(tauri::LogicalSize::new(want, size.height as f64 / scale));
        }
    }
}

#[tauri::command]
fn get_status() -> StatusSnapshot {
    bridge::read()
}

#[tauri::command]
fn install_hooks(app: AppHandle) -> Result<String, String> {
    hooks::install(&app)
}

#[tauri::command]
fn hooks_installed(app: AppHandle) -> bool {
    hooks::is_installed(&app)
}

#[tauri::command]
fn remove_session(id: String) -> Result<(), String> {
    delete_session(&id)
}

#[tauri::command]
fn clear_session_history() -> usize {
    clear_sessions()
}

#[cfg(target_os = "windows")]
fn tray_icon(_state: &str) -> Image<'static> {
    tauri::include_image!("icons/32x32.png").to_owned()
}

#[cfg(not(target_os = "windows"))]
fn tray_icon(state: &str) -> Image<'static> {
    let size = 32u32;
    let color = match state {
        "working" => [250, 204, 21, 255],
        "waiting" => [239, 68, 68, 255],
        "completed" => [34, 197, 94, 255],
        "error" => [239, 68, 68, 255],
        _ => [107, 114, 128, 255],
    };
    let mut rgba = vec![0u8; (size * size * 4) as usize];
    let center = (size as f32 - 1.0) / 2.0;
    for y in 0..size {
        for x in 0..size {
            let distance = ((x as f32 - center).powi(2) + (y as f32 - center).powi(2)).sqrt();
            if distance <= 11.5 {
                let offset = ((y * size + x) * 4) as usize;
                rgba[offset..offset + 4].copy_from_slice(&color);
            }
        }
    }
    Image::new_owned(rgba, size, size)
}

fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.set_focus();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Must be the first plugin: a second launch (shortcut re-click, duplicate
        // autostart entry) exits here and just raises the existing window instead
        // of stacking another lamp.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .invoke_handler(tauri::generate_handler![
            get_status,
            resize_light_to_fit,
            set_hit_regions,
            notify_enabled,
            focus_session,
            presence::get_active_clients,
            install_hooks,
            hooks_installed,
            remove_session,
            clear_session_history,
            reporter::get_team_config,
            reporter::set_team_config,
            reporter::report_status
        ])
        .setup(|app| {
            if std::env::args().any(|a| a == "--enable-autostart") {
                app.autolaunch().enable()?;
            }
            bridge::start().map_err(|e| std::io::Error::other(e.to_string()))?;
            marvis::start();
            zcode::start();
            coze::start();
            trae::start();
            doubao::start();
            codex_local::start();
            workbuddy_local::start();
            reporter::start();
            notify::start(app.handle().clone());

            // Restore the dragged position from the previous run, then keep saving it.
            restore_window_position(app.handle());
            if let Some(window) = app.get_webview_window("main") {
                let save_app = app.handle().clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::Moved(pos) = event {
                        save_window_position(&save_app, pos.x as f64, pos.y as f64);
                    }
                });
            }

            // Hit-test polling: only lamps/panels receive mouse events, everything
            // else passes clicks through to windows underneath. Windows hit-testing
            // cannot be event-driven while ignore_cursor_events is on, hence the poll.
            let hit_app = app.handle().clone();
            thread::spawn(move || {
                // Some(bool) = last state applied; None = unknown (hidden), so the
                // first visible tick always (re)applies a known state.
                let mut ignore: Option<bool> = None;
                loop {
                    thread::sleep(Duration::from_millis(25));
                    let Some(window) = hit_app.get_webview_window("main") else { continue };
                    if !window.is_visible().unwrap_or(false) {
                        ignore = None;
                        continue;
                    }
                    let hit = match (
                        window.outer_position(),
                        window.outer_size(),
                        window.scale_factor(),
                        hit_app.cursor_position(),
                    ) {
                        (Ok(pos), Ok(size), Ok(scale), Ok(cursor)) => {
                            let rel = (cursor.x - pos.x as f64, cursor.y - pos.y as f64);
                            rel.0 >= 0.0
                                && rel.1 >= 0.0
                                && rel.0 <= size.width as f64
                                && rel.1 <= size.height as f64
                                && rects_contain(&hit_rects().lock().unwrap(), scale, rel, 2.0)
                        }
                        // On query failure, keep receiving events rather than
                        // risking an unclickable window.
                        _ => true,
                    };
                    let should_ignore = !hit;
                    if ignore != Some(should_ignore) {
                        let _ = window.set_ignore_cursor_events(should_ignore);
                        ignore = Some(should_ignore);
                    }
                }
            });

            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Regular);

            let status_item = MenuItemBuilder::with_id("status", "AI Traffic Light：空闲")
                .enabled(false)
                .build(app)?;
            let show_item = MenuItemBuilder::with_id("show", "显示悬浮灯").build(app)?;
            let autostart_enabled = app.autolaunch().is_enabled().unwrap_or(false);
            let autostart_item = CheckMenuItemBuilder::with_id("autostart", "开机自启动")
                .checked(autostart_enabled)
                .build(app)?;
            let notify_item = CheckMenuItemBuilder::with_id("notify-toggle", "会话完成通知")
                .checked(notify::load_notify_enabled())
                .build(app)?;
            let quit_item = MenuItemBuilder::with_id("quit", "退出").build(app)?;
            let menu = MenuBuilder::new(app)
                .item(&status_item)
                .separator()
                .item(&show_item)
                .item(&autostart_item)
                .item(&notify_item)
                .separator()
                .item(&quit_item)
                .build()?;

            let menu_autostart_item = autostart_item.clone();
            let menu_notify_item = notify_item.clone();
            let tray = TrayIconBuilder::with_id("main")
                .icon(tray_icon("idle"))
                .tooltip("AI Traffic Light：空闲")
                .menu(&menu)
                .on_menu_event(move |app, event| match event.id().as_ref() {
                    "show" => show_window(app),
                    "setup" => {
                        let _ = hooks::install(app);
                    }
                    "clear-sessions" => {
                        clear_sessions();
                    }
                    "autostart" => {
                        let manager = app.autolaunch();
                        let enabled = manager.is_enabled().unwrap_or(false);
                        let result = if enabled {
                            manager.disable()
                        } else {
                            manager.enable()
                        };
                        if result.is_ok() {
                            let _ = menu_autostart_item.set_checked(!enabled);
                        } else {
                            let _ = menu_autostart_item.set_checked(enabled);
                        }
                    }
                    "notify-toggle" => {
                        let new_value = !notify::enabled();
                        notify::set_enabled(new_value);
                        let _ = menu_notify_item.set_checked(new_value);
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;

            let app_handle = app.handle().clone();
            let status_autostart_item = autostart_item.clone();
            thread::spawn(move || {
              let mut system = sysinfo::System::new();
              let mut was_visible = false;
              loop {
                let clients = presence::refresh(&mut system);
                let visible = !clients.is_empty();
                if visible != was_visible {
                    if let Some(window) = app_handle.get_webview_window("main") {
                        if visible { let _ = window.show(); } else { let _ = window.hide(); }
                    }
                    was_visible = visible;
                }
                thread::sleep(Duration::from_millis(500));
                let snapshot = bridge::read();
                let tooltip = if snapshot.session_count > 0 {
                    format!(
                        "AI Traffic Light：{} | {} 个会话状态",
                        snapshot.label, snapshot.session_count
                    )
                } else {
                    "AI Traffic Light：空闲".to_string()
                };
                let _ = tray.set_tooltip(Some(&tooltip));
                let _ = tray.set_icon(Some(tray_icon(&snapshot.state)));
                let _ = status_item.set_text(tooltip);
                if let Ok(enabled) = app_handle.autolaunch().is_enabled() {
                    let _ = status_autostart_item.set_checked(enabled);
                }

                // Keep the handle alive for the application's lifetime.
                let _ = &app_handle;
              }
            });

            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running AI Traffic Light");
}

#[cfg(test)]
mod tests {
    use super::rects_contain;

    const RECTS: [[f64; 4]; 1] = [[10.0, 20.0, 72.0, 120.0]];

    #[test]
    fn empty_rects_never_contain_cursor() {
        assert!(!rects_contain(&[], 1.0, (50.0, 60.0), 2.0));
    }

    #[test]
    fn cursor_inside_rect_hits() {
        assert!(rects_contain(&RECTS, 1.0, (46.0, 80.0), 2.0));
    }

    #[test]
    fn cursor_outside_rect_misses() {
        assert!(!rects_contain(&RECTS, 1.0, (200.0, 80.0), 2.0));
    }

    #[test]
    fn margin_extends_but_not_beyond() {
        // Logical 10..82 × 20..140, margin 2 → physical 8..84 × 18..142.
        assert!(rects_contain(&RECTS, 1.0, (8.0, 80.0), 2.0));
        assert!(rects_contain(&RECTS, 1.0, (84.0, 80.0), 2.0));
        assert!(rects_contain(&RECTS, 1.0, (46.0, 18.0), 2.0));
        assert!(rects_contain(&RECTS, 1.0, (46.0, 142.0), 2.0));
        assert!(!rects_contain(&RECTS, 1.0, (7.5, 80.0), 2.0));
        assert!(!rects_contain(&RECTS, 1.0, (84.5, 80.0), 2.0));
    }

    #[test]
    fn logical_rects_scale_to_physical() {
        // scale 2 → physical 20..164 × 40..160 (+2 margin).
        assert!(rects_contain(&RECTS, 2.0, (163.0, 41.0), 2.0));
        assert!(rects_contain(&RECTS, 2.0, (19.0, 39.0), 2.0));
        assert!(!rects_contain(&RECTS, 2.0, (170.0, 41.0), 2.0));
        assert!(!rects_contain(&RECTS, 2.0, (17.0, 39.0), 2.0));
        // Inside the unscaled rect but outside the scaled one must miss.
        assert!(!rects_contain(&RECTS, 2.0, (12.0, 25.0), 2.0));
    }
}
