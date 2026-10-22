mod bridge;
mod codex_local;
mod coze;
mod doubao;
mod focus;
mod hooks;
mod marvis;
mod notify;
mod presence;
mod reporter;
mod sessions;
mod trae;
mod workbuddy_local;
mod zcode;

use sessions::{clear_sessions, delete_session, StatusSnapshot};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};
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
        cx >= x - margin_px
            && cx <= x + w + margin_px
            && cy >= y - margin_px
            && cy <= y + h + margin_px
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

// Gap between the window and the primary work area's top-right, in logical px.
// Launch multiplies this by that monitor's own scale factor (physical px).
const DEFAULT_INSET: f64 = 16.0;
const MIN_LOGICAL_W: f64 = 240.0;
const MAX_LOGICAL_W: f64 = 920.0;
const MIN_LOGICAL_H: f64 = 168.0;
const MAX_LOGICAL_H: f64 = 720.0;
const START_LOGICAL_W: f64 = 780.0;
const START_LOGICAL_H: f64 = 480.0;

#[derive(Clone, Copy, Debug, PartialEq)]
struct MonitorBox {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct WindowBox {
    w: f64,
    h: f64,
}

fn clamp_on_monitor(mon: MonitorBox, win: WindowBox, x: f64, y: f64) -> (f64, f64) {
    let (min_x, max_x) = (mon.x + 80.0 - win.w, mon.x + mon.w - 80.0);
    let (min_y, max_y) = (mon.y + 80.0 - win.h, mon.y + mon.h - 80.0);
    let x = if min_x <= max_x {
        x.clamp(min_x, max_x)
    } else {
        mon.x
    };
    let y = if min_y <= max_y {
        y.clamp(min_y, max_y)
    } else {
        mon.y
    };
    (x, y)
}

/// Place from the monitor's top-right. Insets that land in the left or lower
/// half are dropped so a restart stays in that corner.
fn anchored_position(
    mon: MonitorBox,
    win: WindowBox,
    inset_right: f64,
    inset_top: f64,
    default_inset: f64,
) -> (f64, f64) {
    let place = |right: f64, top: f64| (mon.x + mon.w - win.w - right, mon.y + top);
    let (x, y) = place(inset_right, inset_top);
    let stays_top_right = x + win.w >= mon.x + mon.w * 0.55 && y <= mon.y + mon.h * 0.45;
    let (x, y) = if stays_top_right {
        (x, y)
    } else {
        place(default_inset, default_inset)
    };
    clamp_on_monitor(mon, win, x, y)
}

fn right_edge_x(old_x: f64, old_w: f64, new_w: f64) -> f64 {
    old_x + old_w - new_w
}

/// The requested logical size wins. Dividing a stale outer size by scale and
/// writing it back collapsed the beta window toward 16×16.
fn fit_logical_size(width: f64, height: f64) -> Option<(f64, f64)> {
    if !width.is_finite() || !height.is_finite() {
        return None;
    }
    Some((
        width.clamp(MIN_LOGICAL_W, MAX_LOGICAL_W),
        height.clamp(MIN_LOGICAL_H, MAX_LOGICAL_H),
    ))
}

fn should_ignore_cursor(has_regions: bool, inside_window: bool, in_region: bool) -> bool {
    if !has_regions {
        return false;
    }
    if !inside_window {
        return true;
    }
    !in_region
}

/// Top-right of a work area in physical pixels. `scale` is that monitor's
/// DPI scale, not a stale window scale. Origins may be negative.
fn top_right_on_work(
    work: MonitorBox,
    win: WindowBox,
    inset_logical: f64,
    scale: f64,
) -> (f64, f64) {
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    let inset = (inset_logical * scale).round();
    anchored_position(work, win, inset, inset, inset)
}

/// `available` entries are `(full bounds, work area)`. Prefer the primary
/// monitor's work area. If that query failed, use the display that contains
/// the origin, then the first display. Never require a non-negative origin.
fn pick_primary_work(
    primary: Option<MonitorBox>,
    available: &[(MonitorBox, MonitorBox)],
) -> Option<MonitorBox> {
    if let Some(work) = primary {
        return Some(work);
    }
    if let Some((_, work)) = available.iter().copied().find(|(full, _)| {
        full.x <= 0.0 && full.y <= 0.0 && full.x + full.w > 0.0 && full.y + full.h > 0.0
    }) {
        return Some(work);
    }
    available.first().map(|(_, work)| *work)
}

fn monitor_rects(monitor: &tauri::Monitor) -> (MonitorBox, MonitorBox, f64) {
    let pos = monitor.position();
    let size = monitor.size();
    let full = MonitorBox {
        x: pos.x as f64,
        y: pos.y as f64,
        w: size.width as f64,
        h: size.height as f64,
    };
    let area = monitor.work_area();
    let work = MonitorBox {
        x: area.position.x as f64,
        y: area.position.y as f64,
        w: area.size.width as f64,
        h: area.size.height as f64,
    };
    let scale = monitor.scale_factor();
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    (full, work, scale)
}

fn primary_work(window: &tauri::WebviewWindow) -> Option<(MonitorBox, f64)> {
    let primary = window.primary_monitor().ok().flatten();
    let (primary_work, primary_scale) = match primary.as_ref() {
        Some(monitor) => {
            let (_, work, scale) = monitor_rects(monitor);
            (Some(work), Some(scale))
        }
        None => (None, None),
    };
    let mut available = Vec::new();
    if let Ok(monitors) = window.available_monitors() {
        for monitor in monitors {
            let (full, work, _) = monitor_rects(&monitor);
            available.push((full, work));
        }
    }
    let work = pick_primary_work(primary_work, &available)?;
    let scale = primary_scale.unwrap_or_else(|| {
        window
            .available_monitors()
            .ok()
            .and_then(|monitors| {
                monitors.iter().find_map(|monitor| {
                    let (_, candidate, scale) = monitor_rects(monitor);
                    ((candidate.x - work.x).abs() < 2.0 && (candidate.y - work.y).abs() < 2.0)
                        .then_some(scale)
                })
            })
            .unwrap_or(1.0)
    });
    Some((work, scale))
}

// Moves that we issued ourselves. A later Moved event at the same point is
// not a user drag, so launch retries can still correct a restored placement.
static PROGRAMMATIC_POS: OnceLock<Mutex<Option<(i32, i32)>>> = OnceLock::new();
static USER_DRAGGED: AtomicBool = AtomicBool::new(false);
static LAUNCH_AT: OnceLock<Instant> = OnceLock::new();

fn launch_clock() -> Instant {
    *LAUNCH_AT.get_or_init(Instant::now)
}

fn note_programmatic_pos(x: f64, y: f64) {
    if let Ok(mut guard) = PROGRAMMATIC_POS.get_or_init(|| Mutex::new(None)).lock() {
        *guard = Some((x.round() as i32, y.round() as i32));
    }
}

fn is_programmatic_move(x: f64, y: f64) -> bool {
    let Some(slot) = PROGRAMMATIC_POS.get() else {
        return false;
    };
    let Ok(guard) = slot.lock() else {
        return false;
    };
    let Some((px, py)) = *guard else {
        return false;
    };
    (x.round() as i32 - px).abs() <= 4 && (y.round() as i32 - py).abs() <= 4
}

fn pointer_is_down() -> bool {
    #[cfg(windows)]
    {
        // High bit set means the left button is down. A placement restore
        // moves the window without a button, so it must not count as a drag.
        unsafe { winapi::um::winuser::GetAsyncKeyState(0x01) < 0 }
    }
    #[cfg(not(windows))]
    {
        true
    }
}

fn note_user_drag(x: f64, y: f64) {
    if is_programmatic_move(x, y) || !pointer_is_down() {
        return;
    }
    USER_DRAGGED.store(true, Ordering::Relaxed);
}

fn launch_should_place() -> bool {
    !USER_DRAGGED.load(Ordering::Relaxed) && launch_clock().elapsed() < Duration::from_millis(1600)
}

fn main_window_hidden(app: &AppHandle) -> bool {
    app.get_webview_window("main")
        .and_then(|window| window.is_visible().ok())
        .map(|visible| !visible)
        .unwrap_or(true)
}

fn launch_physical_size(scale: f64, reset_to_start: bool) -> (f64, f64) {
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    if !reset_to_start {
        if let Some(slot) = LAST_FIT.get() {
            if let Ok(fitted) = slot.lock() {
                if fitted.0 >= MIN_LOGICAL_W as i32 && fitted.1 >= MIN_LOGICAL_H as i32 {
                    return (fitted.0 as f64 * scale, fitted.1 as f64 * scale);
                }
            }
        }
    }
    (START_LOGICAL_W * scale, START_LOGICAL_H * scale)
}

fn place_primary_top_right(
    window: &tauri::WebviewWindow,
    reset_to_start: bool,
) -> Option<(i32, i32, i32, i32)> {
    let (work, scale) = primary_work(window)?;
    let (width, height) = launch_physical_size(scale, reset_to_start);
    if !width.is_finite() || !height.is_finite() || width < 1.0 || height < 1.0 {
        return None;
    }
    let (x, y) = top_right_on_work(
        work,
        WindowBox {
            w: width,
            h: height,
        },
        DEFAULT_INSET,
        scale,
    );
    if !x.is_finite() || !y.is_finite() {
        return None;
    }
    let _ = window.set_size(tauri::PhysicalSize::new(
        width.round() as u32,
        height.round() as u32,
    ));
    note_programmatic_pos(x, y);
    let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
    Some((
        x.round() as i32,
        y.round() as i32,
        width.round() as i32,
        height.round() as i32,
    ))
}

/// Show the main window. `place` pins it to the primary work area's top-right.
/// Positioning failure still shows and focuses. The frontend `window_ready`
/// invoke is not a gate: the page may never load in a release build.
fn present_main_window(app: &AppHandle, place: bool, reset_to_start: bool) {
    let Some(window) = app.get_webview_window("main") else {
        return;
    };
    let needs_focus = place || window.is_visible().ok() != Some(true);
    let placed = if place {
        place_primary_top_right(&window, reset_to_start)
    } else {
        None
    };
    let _ = window.unminimize();
    let _ = window.show();
    #[cfg(windows)]
    if let Ok(hwnd) = window.hwnd() {
        // tao's Show path calls SetWindowPos(SWP_FRAMECHANGED) without
        // SWP_SHOWWINDOW, which clears WS_VISIBLE on a window that was born
        // hidden. Force the bit after that style refresh.
        force_show_win32(hwnd.0, placed, needs_focus);
    }
    #[cfg(not(windows))]
    let _ = placed;
    if needs_focus {
        let _ = window.set_focus();
    }
}

fn schedule_present(app: AppHandle, delay_ms: u64) {
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(delay_ms));
        let queued = app.clone();
        let for_task = queued.clone();
        if app
            .run_on_main_thread(move || {
                let place = main_window_hidden(&for_task) || launch_should_place();
                present_main_window(&for_task, place, false);
            })
            .is_err()
        {
            let _ = queued.get_webview_window("main").map(|window| {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            });
        }
    });
}

#[cfg(windows)]
fn force_show_win32(raw: *mut std::ffi::c_void, place: Option<(i32, i32, i32, i32)>, focus: bool) {
    use winapi::shared::minwindef::LPARAM;
    use winapi::shared::windef::HWND;
    use winapi::um::winuser::{
        EnumChildWindows, IsIconic, SetForegroundWindow, SetWindowPos, ShowWindow, HWND_TOPMOST,
        SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_RESTORE, SW_SHOW, SW_SHOWNA,
    };

    let hwnd = raw as HWND;
    unsafe {
        if IsIconic(hwnd) != 0 {
            ShowWindow(hwnd, SW_RESTORE);
        }
        ShowWindow(hwnd, if focus { SW_SHOW } else { SW_SHOWNA });
        if let Some((x, y, width, height)) = place {
            // After ShowWindow, so a restored WINDOWPLACEMENT cannot put the
            // old rect (or SW_HIDE) back.
            let mut flags = SWP_SHOWWINDOW;
            if !focus {
                flags |= SWP_NOACTIVATE;
            }
            SetWindowPos(hwnd, HWND_TOPMOST, x, y, width.max(1), height.max(1), flags);
        }
        EnumChildWindows(hwnd, Some(show_child_window), 0 as LPARAM);
        if focus {
            SetForegroundWindow(hwnd);
        }
    }
}

#[cfg(windows)]
unsafe extern "system" fn show_child_window(
    hwnd: winapi::shared::windef::HWND,
    _: winapi::shared::minwindef::LPARAM,
) -> i32 {
    // The WebView2 child is created without WS_VISIBLE when the window config
    // says visible:false. WebviewWindow::show does not flip that child.
    winapi::um::winuser::ShowWindow(hwnd, winapi::um::winuser::SW_SHOW);
    1
}

static LAST_FIT: OnceLock<Mutex<(i32, i32)>> = OnceLock::new();

// Width and height follow the visible lamps plus the open session list.
// Each distinct request is applied once; the next size is never derived from
// the current outer size, which is what shrank the window on every tick.
#[tauri::command]
fn resize_light_to_fit(window: tauri::WebviewWindow, width: f64, height: f64) {
    let Some((want_w, want_h)) = fit_logical_size(width, height) else {
        return;
    };
    let key = (want_w.round() as i32, want_h.round() as i32);
    let slot = LAST_FIT.get_or_init(|| Mutex::new((0, 0)));
    if let Ok(mut previous) = slot.lock() {
        if (previous.0 - key.0).abs() <= 4 && (previous.1 - key.1).abs() <= 4 {
            return;
        }
        *previous = key;
    }
    let scale = window.scale_factor().unwrap_or(1.0).max(0.5);
    let pos = window.outer_position().ok();
    let old_w_phys = window
        .outer_size()
        .ok()
        .filter(|size| size.width >= 64)
        .map(|size| size.width as f64)
        .unwrap_or(want_w * scale);
    let _ = window.set_size(tauri::LogicalSize::new(want_w, want_h));
    if let Some(pos) = pos {
        let new_x = right_edge_x(pos.x as f64, old_w_phys, want_w * scale);
        note_programmatic_pos(new_x, pos.y as f64);
        let _ = window.set_position(tauri::PhysicalPosition::new(new_x, pos.y as f64));
    }
}

#[tauri::command]
fn get_status() -> StatusSnapshot {
    bridge::read()
}

#[derive(serde::Serialize)]
struct PromptPage {
    prompts: Vec<String>,
    note: String,
}

#[tauri::command]
fn session_user_prompts(client: String, id: String) -> Result<PromptPage, String> {
    if client != "codex" {
        return Ok(PromptPage {
            prompts: Vec::new(),
            note: "该客户端暂不能读取提问原文，列表里仍可看到标题和状态。".to_string(),
        });
    }
    let prompts = codex_local::user_prompts(&id)?;
    let note = if prompts.is_empty() {
        "这条会话里没有找到你的提问。"
    } else {
        "只显示你的提问，不含 AI 回复。"
    };
    Ok(PromptPage {
        prompts,
        note: note.to_string(),
    })
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
    let place = main_window_hidden(app) || launch_should_place();
    present_main_window(app, place, false);
}

// Page load is not required. If this never arrives (missing dist, custom
// protocol, or a blank webview), the launch timers still show the window.
#[tauri::command]
fn window_ready(app: AppHandle) {
    let queued = app.clone();
    if app
        .run_on_main_thread(move || {
            let place = main_window_hidden(&queued) || launch_should_place();
            present_main_window(&queued, place, false);
        })
        .is_err()
    {
        let _ = app.get_webview_window("main").map(|window| {
            let _ = window.unminimize();
            let _ = window.show();
            let _ = window.set_focus();
        });
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Must be the first plugin: a second launch (shortcut re-click, duplicate
        // autostart entry) exits here and just raises the existing window instead
        // of stacking another lamp.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            let queued = app.clone();
            let _ = app.run_on_main_thread(move || {
                let place = main_window_hidden(&queued) || launch_should_place();
                present_main_window(&queued, place, false);
            });
        }))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .invoke_handler(tauri::generate_handler![
            get_status,
            session_user_prompts,
            resize_light_to_fit,
            window_ready,
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

            // Distinguish a user drag from the placement we apply below.
            // Showing does not wait for the webview's window_ready invoke.
            let _ = launch_clock();
            if let Some(window) = app.get_webview_window("main") {
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::Moved(pos) = event {
                        note_user_drag(pos.x as f64, pos.y as f64);
                    }
                });
            }
            present_main_window(app.handle(), true, true);
            schedule_present(app.handle().clone(), 300);
            schedule_present(app.handle().clone(), 1200);
            // Late enough to beat a WINDOWPLACEMENT restore, still shows if
            // the page never called window_ready. Does not steal focus when
            // the window is already visible and the user has dragged it.
            schedule_present(app.handle().clone(), 2500);

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
                    let Some(window) = hit_app.get_webview_window("main") else {
                        continue;
                    };
                    if !window.is_visible().unwrap_or(false) {
                        ignore = None;
                        continue;
                    }
                    let rects = hit_rects().lock().unwrap().clone();
                    let should_ignore = match (
                        window.outer_position(),
                        window.outer_size(),
                        window.scale_factor(),
                        hit_app.cursor_position(),
                    ) {
                        (Ok(pos), Ok(size), Ok(scale), Ok(cursor)) => {
                            let rel = (cursor.x - pos.x as f64, cursor.y - pos.y as f64);
                            let inside = rel.0 >= 0.0
                                && rel.1 >= 0.0
                                && rel.0 <= size.width as f64
                                && rel.1 <= size.height as f64;
                            let in_region = inside && rects_contain(&rects, scale, rel, 2.0);
                            // Empty regions stay clickable. A 16×16 window with no
                            // measured lamps used to ignore every click.
                            should_ignore_cursor(!rects.is_empty(), inside, in_region)
                        }
                        // On query failure, keep receiving events rather than
                        // risking an unclickable window.
                        _ => false,
                    };
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
                loop {
                    // Presence still feeds the tray and team report. It must not hide
                    // the lamp: sessions are readable when the client process name
                    // does not match, and the window is meant to stay on screen.
                    presence::refresh(&mut system);
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
        .build(tauri::generate_context!())
        .expect("error while building AI Traffic Light")
        .run(|app, event| {
            if matches!(event, tauri::RunEvent::Ready) {
                // setup() already ran inside Ready. Show once more in case
                // Windows restored a hidden placement while startup finished.
                let place = main_window_hidden(app) || launch_should_place();
                present_main_window(app, place, false);
            }
        });
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

    #[test]
    fn empty_hit_regions_stay_clickable() {
        assert!(!super::should_ignore_cursor(false, true, false));
        assert!(!super::should_ignore_cursor(false, false, false));
    }

    #[test]
    fn transparent_gaps_pass_clicks_once_regions_exist() {
        assert!(super::should_ignore_cursor(true, true, false));
        assert!(!super::should_ignore_cursor(true, true, true));
        assert!(super::should_ignore_cursor(true, false, false));
    }

    #[test]
    fn fit_size_ignores_collapsed_outer_measurement() {
        let fitted = super::fit_logical_size(720.0, 420.0).unwrap();
        assert_eq!(fitted, (720.0, 420.0));
        assert!(fitted.0 >= 240.0 && fitted.1 >= 168.0);
        assert!(super::fit_logical_size(f64::NAN, 400.0).is_none());
        let tiny = super::fit_logical_size(16.0, 16.0).unwrap();
        assert!(tiny.0 >= 240.0 && tiny.1 >= 168.0);
    }

    #[test]
    fn resize_keeps_the_right_edge() {
        assert_eq!(super::right_edge_x(1000.0, 400.0, 700.0), 700.0);
    }

    #[test]
    fn startup_anchor_stays_top_right() {
        let mon = super::MonitorBox {
            x: 0.0,
            y: 0.0,
            w: 1920.0,
            h: 1080.0,
        };
        let win = super::WindowBox { w: 800.0, h: 400.0 };
        let (x, y) = super::anchored_position(mon, win, 40.0, 24.0, 16.0);
        assert_eq!((x, y), (1080.0, 24.0));
        let (x, y) = super::anchored_position(mon, win, 1500.0, 900.0, 16.0);
        assert_eq!((x, y), (1104.0, 16.0));
    }

    #[test]
    fn primary_work_area_top_right_handles_dpi_and_negative_origins() {
        // Primary 1920×1080, taskbar 40px, 100% DPI, startup window 780×480.
        let work = super::MonitorBox {
            x: 0.0,
            y: 0.0,
            w: 1920.0,
            h: 1040.0,
        };
        let win = super::WindowBox { w: 780.0, h: 480.0 };
        assert_eq!(
            super::top_right_on_work(work, win, 16.0, 1.0),
            (1124.0, 16.0)
        );

        // 150% DPI: logical 780×480 is 1170×720 physical, inset 24.
        let hidpi = super::WindowBox {
            w: 1170.0,
            h: 720.0,
        };
        assert_eq!(
            super::top_right_on_work(work, hidpi, 16.0, 1.5),
            (726.0, 24.0)
        );

        // Display left of and above the origin. Do not clamp to >= 0.
        let off = super::MonitorBox {
            x: -1720.0,
            y: -2271.0,
            w: 2560.0,
            h: 1440.0,
        };
        let scaled = super::WindowBox { w: 975.0, h: 600.0 };
        assert_eq!(
            super::top_right_on_work(off, scaled, 16.0, 1.25),
            (-155.0, -2251.0)
        );

        // Vertical monitor above the origin (840,-1920, 1080×1920).
        let tall = super::MonitorBox {
            x: 840.0,
            y: -1920.0,
            w: 1080.0,
            h: 1920.0,
        };
        assert_eq!(
            super::top_right_on_work(tall, win, 16.0, 1.0),
            (1124.0, -1904.0)
        );
    }

    #[test]
    fn hidden_creation_rect_is_not_the_primary_top_right() {
        // EnumWindows on the broken launch: vis=false at 156,156 size 1440×753.
        let work = super::MonitorBox {
            x: 0.0,
            y: 0.0,
            w: 1920.0,
            h: 1080.0,
        };
        let created = super::WindowBox {
            w: 1440.0,
            h: 753.0,
        };
        let (x, y) = super::top_right_on_work(work, created, 16.0, 1.0);
        assert_ne!((x, y), (156.0, 156.0));
        assert!(x > 156.0 && y < 156.0);
    }

    #[test]
    fn primary_work_area_wins_over_other_monitors() {
        let right = (
            super::MonitorBox {
                x: 1920.0,
                y: -831.0,
                w: 1920.0,
                h: 1080.0,
            },
            super::MonitorBox {
                x: 1920.0,
                y: -831.0,
                w: 1920.0,
                h: 1040.0,
            },
        );
        let left = (
            super::MonitorBox {
                x: -1720.0,
                y: -2271.0,
                w: 2560.0,
                h: 1440.0,
            },
            super::MonitorBox {
                x: -1720.0,
                y: -2271.0,
                w: 2560.0,
                h: 1440.0,
            },
        );
        let vertical = (
            super::MonitorBox {
                x: 840.0,
                y: -1920.0,
                w: 1080.0,
                h: 1920.0,
            },
            super::MonitorBox {
                x: 840.0,
                y: -1920.0,
                w: 1080.0,
                h: 1880.0,
            },
        );
        let primary_full = super::MonitorBox {
            x: 0.0,
            y: 0.0,
            w: 1920.0,
            h: 1080.0,
        };
        let primary_work = super::MonitorBox {
            x: 0.0,
            y: 0.0,
            w: 1920.0,
            h: 1040.0,
        };
        let available = [right, left, vertical, (primary_full, primary_work)];
        assert_eq!(
            super::pick_primary_work(Some(primary_work), &available),
            Some(primary_work)
        );
        // primary_monitor() failed: still the display that contains (0, 0),
        // not whichever monitor was enumerated first.
        assert_eq!(
            super::pick_primary_work(None, &available),
            Some(primary_work)
        );
    }
}
