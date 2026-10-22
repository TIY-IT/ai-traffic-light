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
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
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

fn finite_or(value: f64, fallback: f64) -> f64 {
    if value.is_finite() {
        value
    } else if fallback.is_finite() {
        fallback
    } else {
        0.0
    }
}

/// `f64::clamp` panics when a bound is NaN or min > max. Monitor origins are
/// negative on this machine, and a failed query must not take the process down.
fn safe_clamp(value: f64, min: f64, max: f64, fallback: f64) -> f64 {
    let value = finite_or(value, fallback);
    if !min.is_finite() || !max.is_finite() || min > max {
        return value;
    }
    value.clamp(min, max)
}

fn to_i32_px(value: f64) -> i32 {
    let value = finite_or(value, 0.0).round();
    if value >= i32::MAX as f64 {
        i32::MAX
    } else if value <= i32::MIN as f64 {
        i32::MIN
    } else {
        value as i32
    }
}

fn to_u32_px(value: f64) -> u32 {
    let value = finite_or(value, 1.0).round();
    if value <= 1.0 {
        1
    } else if value >= u32::MAX as f64 {
        u32::MAX
    } else {
        value as u32
    }
}

fn clamp_on_monitor(mon: MonitorBox, win: WindowBox, x: f64, y: f64) -> (f64, f64) {
    let min_x = mon.x + 80.0 - win.w;
    let max_x = mon.x + mon.w - 80.0;
    let min_y = mon.y + 80.0 - win.h;
    let max_y = mon.y + mon.h - 80.0;
    (
        safe_clamp(x, min_x, max_x, mon.x),
        safe_clamp(y, min_y, max_y, mon.y),
    )
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
static QUITTING: AtomicBool = AtomicBool::new(false);
static REBUILDS: AtomicU8 = AtomicU8::new(0);
static LOG_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

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

fn log_path() -> PathBuf {
    #[cfg(windows)]
    {
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            let local = local.trim();
            if !local.is_empty() {
                return PathBuf::from(local)
                    .join("com.pcj1997.ai-traffic-light")
                    .join("logs")
                    .join("app.log");
            }
        }
    }
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".ai-traffic-light")
        .join("logs")
        .join("app.log")
}

fn log_line(message: &str) {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or(0);
    let line = format!("{millis} {message}\n");
    let Ok(guard) = LOG_LOCK.get_or_init(|| Mutex::new(())).lock() else {
        return;
    };
    let path = log_path();
    let write = (|| -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)?;
        file.write_all(line.as_bytes())?;
        Ok(())
    })();
    drop(guard);
    let _ = write;
}

fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|site| format!("{}:{}:{}", site.file(), site.line(), site.column()))
            .unwrap_or_else(|| "unknown".to_string());
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|text| (*text).to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "non-string panic payload".to_string());
        // Write the file directly. `log_line` takes LOG_LOCK, and the panicking
        // thread may already hold it — locking again here would deadlock.
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis())
            .unwrap_or(0);
        let line = format!("{millis} PANIC at {location}: {message}\n");
        let path = log_path();
        let _ = (|| -> std::io::Result<()> {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)?;
            file.write_all(line.as_bytes())?;
            Ok(())
        })();
        previous(info);
    }));
}

fn place_primary_top_right(
    window: &tauri::WebviewWindow,
    reset_to_start: bool,
) -> Option<(i32, i32, i32, i32)> {
    log_monitors(window);
    let (work, scale) = match primary_work(window) {
        Some(found) => found,
        None => {
            log_line("place skipped: no monitor");
            return None;
        }
    };
    log_line(&format!(
        "primary work x={} y={} w={} h={} scale={scale}",
        work.x, work.y, work.w, work.h
    ));
    let (width, height) = launch_physical_size(scale, reset_to_start);
    if !width.is_finite() || !height.is_finite() || width < 1.0 || height < 1.0 {
        log_line(&format!("place skipped: bad size {width}x{height}"));
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
        log_line(&format!("place skipped: bad origin {x},{y}"));
        return None;
    }
    let px = to_u32_px(width);
    let py = to_u32_px(height);
    let ix = to_i32_px(x);
    let iy = to_i32_px(y);
    match window.set_size(tauri::PhysicalSize::new(px, py)) {
        Ok(()) => log_line(&format!("set_size {px}x{py}")),
        Err(error) => log_line(&format!("set_size error: {error}")),
    }
    note_programmatic_pos(ix as f64, iy as f64);
    match window.set_position(tauri::PhysicalPosition::new(ix, iy)) {
        Ok(()) => log_line(&format!("set_position {ix},{iy}")),
        Err(error) => log_line(&format!("set_position error: {error}")),
    }
    Some((ix, iy, to_i32_px(width), to_i32_px(height)))
}

fn log_monitors(window: &tauri::WebviewWindow) {
    match window.available_monitors() {
        Ok(monitors) => {
            for (index, monitor) in monitors.iter().enumerate() {
                let (full, work, scale) = monitor_rects(monitor);
                log_line(&format!(
                    "monitor[{index}] name={:?} scale={scale} full={},{} {}x{} work={},{} {}x{}",
                    monitor.name(),
                    full.x,
                    full.y,
                    full.w,
                    full.h,
                    work.x,
                    work.y,
                    work.w,
                    work.h
                ));
            }
        }
        Err(error) => log_line(&format!("available_monitors error: {error}")),
    }
    match window.primary_monitor() {
        Ok(Some(monitor)) => log_line(&format!("primary name={:?}", monitor.name())),
        Ok(None) => log_line("primary monitor: none"),
        Err(error) => log_line(&format!("primary_monitor error: {error}")),
    }
}

fn open_configured_window(app: &AppHandle) -> Result<tauri::WebviewWindow, String> {
    let config = app
        .config()
        .app
        .windows
        .iter()
        .find(|window| window.label == "main")
        .cloned()
        .ok_or_else(|| "tauri.conf.json has no window labeled main".to_string())?;
    log_line(&format!(
        "from_config label={} create={} visible={} {}x{} at {:?},{:?}",
        config.label,
        config.create,
        config.visible,
        config.width,
        config.height,
        config.x,
        config.y
    ));
    let window = tauri::WebviewWindowBuilder::from_config(app, &config)
        .map_err(|error| error.to_string())?
        .visible(true)
        .build()
        .map_err(|error| error.to_string())?;
    Ok(window)
}

fn open_fallback_window(app: &AppHandle) -> Result<tauri::WebviewWindow, String> {
    log_line("creating fallback WebviewWindowBuilder");
    tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::App("index.html".into()))
        .title("AI Traffic Light")
        .inner_size(START_LOGICAL_W, START_LOGICAL_H)
        .min_inner_size(MIN_LOGICAL_W, MIN_LOGICAL_H)
        .decorations(false)
        .transparent(true)
        .always_on_top(true)
        .resizable(false)
        .skip_taskbar(true)
        .shadow(false)
        .visible(true)
        .focused(true)
        .build()
        .map_err(|error| error.to_string())
}

fn ensure_main_window(app: &AppHandle) -> Result<(), String> {
    if app.get_webview_window("main").is_some() {
        log_line("main window already exists");
        return Ok(());
    }
    log_line("main window missing, creating");
    // The closure returns Result<(), String>, not the window: WebviewWindow is
    // not UnwindSafe, and catch_unwind requires the success type to be.
    let configured = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        open_configured_window(app).map(|_| ())
    }));
    match configured {
        Ok(Ok(())) => {
            log_line("configured window created");
            return Ok(());
        }
        Ok(Err(error)) => log_line(&format!("configured window failed: {error}")),
        Err(_) => log_line("configured window panicked"),
    }
    if app.get_webview_window("main").is_some() {
        log_line("main window appeared after configured attempt");
        return Ok(());
    }
    let fallback = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        open_fallback_window(app).map(|_| ())
    }));
    match fallback {
        Ok(Ok(())) => {
            log_line("fallback window created");
            Ok(())
        }
        Ok(Err(error)) => {
            log_line(&format!("fallback window failed: {error}"));
            Err(error)
        }
        Err(_) => {
            log_line("fallback window panicked");
            Err("fallback window panicked".to_string())
        }
    }
}

fn log_window_state(window: &tauri::WebviewWindow, stage: &str) {
    let visible = window
        .is_visible()
        .map(|value| value.to_string())
        .unwrap_or_else(|error| format!("err {error}"));
    let position = window
        .outer_position()
        .map(|pos| format!("{},{}", pos.x, pos.y))
        .unwrap_or_else(|error| format!("err {error}"));
    let size = window
        .outer_size()
        .map(|size| format!("{}x{}", size.width, size.height))
        .unwrap_or_else(|error| format!("err {error}"));
    log_line(&format!(
        "{stage}: visible={visible} pos={position} size={size}"
    ));
}

/// Show the main window. `place` pins it to the primary work area's top-right.
/// Positioning failure still shows and focuses. `native` is the Win32 show of
/// the parent only; child WebView2 HWNDs are left alone because showing them
/// during startup re-enters the controller and can destroy the parent.
/// The frontend `window_ready` invoke is not a gate.
fn present_main_window(app: &AppHandle, place: bool, reset_to_start: bool, native: bool) {
    if app.get_webview_window("main").is_none() {
        if let Err(error) = ensure_main_window(app) {
            log_line(&format!("present aborted: {error}"));
            return;
        }
    }
    let Some(window) = app.get_webview_window("main") else {
        log_line("present aborted: main still missing");
        return;
    };
    let needs_focus = place || window.is_visible().ok() != Some(true);
    let placed = if place {
        place_primary_top_right(&window, reset_to_start)
    } else {
        None
    };
    match window.unminimize() {
        Ok(()) => log_line("unminimize ok"),
        Err(error) => log_line(&format!("unminimize error: {error}")),
    }
    match window.show() {
        Ok(()) => log_line("show ok"),
        Err(error) => log_line(&format!("show error: {error}")),
    }
    #[cfg(windows)]
    if native {
        match window.hwnd() {
            Ok(hwnd) => force_show_win32(hwnd.0, placed, needs_focus),
            Err(error) => log_line(&format!("hwnd error: {error}")),
        }
    }
    #[cfg(not(windows))]
    let _ = (native, placed);
    if needs_focus {
        match window.set_focus() {
            Ok(()) => log_line("set_focus ok"),
            Err(error) => log_line(&format!("set_focus error: {error}")),
        }
    }
    log_window_state(
        &window,
        if native {
            "after native show"
        } else {
            "after tauri show"
        },
    );
}

fn schedule_present(app: AppHandle, delay_ms: u64) {
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(delay_ms));
        let queued = app.clone();
        let for_task = queued.clone();
        if app
            .run_on_main_thread(move || {
                log_line("deferred present");
                let place = main_window_hidden(&for_task) || launch_should_place();
                present_main_window(&for_task, place, false, true);
            })
            .is_err()
        {
            log_line("run_on_main_thread failed; showing without native call");
            let _ = queued.get_webview_window("main").map(|window| {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            });
        }
    });
}

fn schedule_recreate(app: AppHandle) {
    if QUITTING.load(Ordering::Relaxed) {
        return;
    }
    if REBUILDS.fetch_add(1, Ordering::Relaxed) >= 2 {
        log_line("main destroyed; not recreating again");
        return;
    }
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(200));
        let queued = app.clone();
        let _ = app.run_on_main_thread(move || {
            log_line("recreate main after destroy");
            let _ = ensure_main_window(&queued);
            present_main_window(&queued, true, true, true);
        });
    });
}

#[cfg(windows)]
fn force_show_win32(raw: *mut std::ffi::c_void, place: Option<(i32, i32, i32, i32)>, focus: bool) {
    use winapi::shared::windef::HWND;
    use winapi::um::winuser::{
        IsIconic, IsWindow, SetForegroundWindow, SetWindowPos, ShowWindow, HWND_TOPMOST,
        SWP_NOACTIVATE, SWP_SHOWWINDOW, SW_RESTORE, SW_SHOW, SW_SHOWNA,
    };

    if raw.is_null() {
        log_line("native show skipped: null hwnd");
        return;
    }
    let hwnd = raw as HWND;
    unsafe {
        if IsWindow(hwnd) == 0 {
            log_line("native show skipped: hwnd is not a window");
            return;
        }
        if IsIconic(hwnd) != 0 {
            ShowWindow(hwnd, SW_RESTORE);
            log_line("native SW_RESTORE");
        }
        let shown = ShowWindow(hwnd, if focus { SW_SHOW } else { SW_SHOWNA });
        log_line(&format!("native ShowWindow previous_visible={shown}"));
        if let Some((x, y, width, height)) = place {
            let mut flags = SWP_SHOWWINDOW;
            if !focus {
                flags |= SWP_NOACTIVATE;
            }
            let ok = SetWindowPos(hwnd, HWND_TOPMOST, x, y, width.max(1), height.max(1), flags);
            log_line(&format!(
                "native SetWindowPos {x},{y} {}x{} ok={}",
                width.max(1),
                height.max(1),
                ok != 0
            ));
        }
        if focus {
            let ok = SetForegroundWindow(hwnd);
            log_line(&format!("native SetForegroundWindow ok={}", ok != 0));
        }
    }
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
    present_main_window(app, place, false, true);
}

// Page load is not required. If this never arrives (missing dist, custom
// protocol, or a blank webview), the launch timers still show the window.
#[tauri::command]
fn window_ready(app: AppHandle) {
    let queued = app.clone();
    if app
        .run_on_main_thread(move || {
            let place = main_window_hidden(&queued) || launch_should_place();
            present_main_window(&queued, place, false, true);
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
    install_panic_hook();
    log_line(&format!(
        "start cwd={:?} exe={:?} args={:?}",
        std::env::current_dir().ok(),
        std::env::current_exe().ok(),
        std::env::args().collect::<Vec<_>>()
    ));
    log_line(&format!("log file {}", log_path().display()));
    let builder = tauri::Builder::default()
        // Must be the first plugin: a second launch (shortcut re-click, duplicate
        // autostart entry) exits here and just raises the existing window instead
        // of stacking another lamp.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            let queued = app.clone();
            let _ = app.run_on_main_thread(move || {
                let place = main_window_hidden(&queued) || launch_should_place();
                present_main_window(&queued, place, false, true);
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
            log_line("setup begin");
            if std::env::args().any(|a| a == "--enable-autostart") {
                match app.autolaunch().enable() {
                    Ok(()) => log_line("autostart enabled"),
                    Err(error) => log_line(&format!("autostart error: {error}")),
                }
            }
            if let Err(error) = bridge::start() {
                log_line(&format!("bridge start error: {error}"));
                return Err(std::io::Error::other(error.to_string()).into());
            }
            marvis::start();
            zcode::start();
            coze::start();
            trae::start();
            doubao::start();
            codex_local::start();
            workbuddy_local::start();
            reporter::start();
            notify::start(app.handle().clone());

            // The config window is not auto-created (`create: false`). Build it
            // here so a failure is logged and the fallback builder can run,
            // instead of Tauri panicking before this hook.
            let _ = launch_clock();
            if let Err(error) = ensure_main_window(app.handle()) {
                log_line(&format!("setup ensure failed: {error}"));
            }
            if let Some(window) = app.get_webview_window("main") {
                let rebuild = app.handle().clone();
                window.on_window_event(move |event| match event {
                    tauri::WindowEvent::Moved(pos) => note_user_drag(pos.x as f64, pos.y as f64),
                    tauri::WindowEvent::CloseRequested { api, .. } => {
                        log_line("main CloseRequested");
                        if !QUITTING.load(Ordering::Relaxed)
                            && launch_clock().elapsed() < Duration::from_secs(8)
                        {
                            api.prevent_close();
                            log_line("prevented CloseRequested during launch");
                        }
                    }
                    tauri::WindowEvent::Destroyed => {
                        log_line("main Destroyed");
                        schedule_recreate(rebuild.clone());
                    }
                    _ => {}
                });
            } else {
                log_line("setup: main window still absent");
            }
            // Tauri show only. Native ShowWindow is deferred so it does not
            // re-enter WebView2 while the controller is still starting.
            present_main_window(app.handle(), true, true, false);
            schedule_present(app.handle().clone(), 100);
            schedule_present(app.handle().clone(), 300);
            schedule_present(app.handle().clone(), 1200);
            schedule_present(app.handle().clone(), 2500);
            log_line("setup window scheduled");

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
                    "quit" => {
                        QUITTING.store(true, Ordering::Relaxed);
                        log_line("quit from tray");
                        app.exit(0);
                    }
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

            log_line("setup end");
            Ok(())
        });
    match builder.build(tauri::generate_context!()) {
        Ok(app) => {
            log_line("builder ok");
            app.run(|app, event| match event {
                tauri::RunEvent::Ready => {
                    log_line("RunEvent::Ready");
                    let place = main_window_hidden(app) || launch_should_place();
                    present_main_window(app, place, false, false);
                }
                tauri::RunEvent::ExitRequested { code, api, .. } => {
                    log_line(&format!("ExitRequested code={code:?}"));
                    // Last-window Destroyed asks to exit with code None. Tray quit
                    // calls app.exit(0), which arrives as Some. Keep the process
                    // up so a destroyed main window can be created again.
                    if code.is_none() && !QUITTING.load(Ordering::Relaxed) {
                        api.prevent_exit();
                        log_line("prevented exit after main window closed");
                    }
                }
                tauri::RunEvent::Exit => log_line("Exit"),
                _ => {}
            });
        }
        Err(error) => log_line(&format!("builder failed: {error}")),
    }
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

    #[test]
    fn clamp_survives_nan_and_negative_monitors() {
        let mon = super::MonitorBox {
            x: -1720.0,
            y: -2271.0,
            w: 2560.0,
            h: 1440.0,
        };
        let (x, y) = super::clamp_on_monitor(
            mon,
            super::WindowBox {
                w: f64::NAN,
                h: f64::NAN,
            },
            f64::NAN,
            f64::NAN,
        );
        assert!(x.is_finite() && y.is_finite());
        assert_eq!(super::to_i32_px(-1720.4), -1720);
        assert_eq!(super::to_i32_px(f64::NAN), 0);
        assert_eq!(super::to_u32_px(-5.0), 1);
    }
}
