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
const MIN_LOGICAL_W: f64 = 72.0;
const MAX_LOGICAL_W: f64 = 1400.0;
const MIN_LOGICAL_H: f64 = 120.0;
const MAX_LOGICAL_H: f64 = 980.0;
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
// Set when no client is running or freshly stopped. Startup presents must not
// undo that hide. Tray "显示悬浮灯" clears it and sets FORCE_VISIBLE.
static PRESENCE_HIDDEN: AtomicBool = AtomicBool::new(false);
static FORCE_VISIBLE: AtomicBool = AtomicBool::new(false);
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

fn local_log_path() -> Option<PathBuf> {
    let local = std::env::var_os("LOCALAPPDATA")?;
    if local.is_empty() {
        return None;
    }
    Some(
        PathBuf::from(local)
            .join("com.pcj1997.ai-traffic-light")
            .join("logs")
            .join("app.log"),
    )
}

fn exe_log_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    Some(exe.parent()?.join("ai-traffic-light.log"))
}

fn temp_log_path() -> Option<PathBuf> {
    let temp = std::env::var_os("TEMP").or_else(|| std::env::var_os("TMP"))?;
    if temp.is_empty() {
        return None;
    }
    Some(PathBuf::from(temp).join("ai-traffic-light.log"))
}

/// `append(true)` alone fails to create a new file on some Windows std builds.
/// Read access is required for that create+append combination.
fn write_log_line(path: &std::path::Path, line: &[u8]) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        if parent.components().next().is_some() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .read(true)
        .open(path)?;
    file.write_all(line)?;
    file.flush()?;
    let _ = file.sync_data();
    Ok(())
}

pub(crate) fn log_line(message: &str) {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis())
        .unwrap_or(0);
    let line = format!("{millis} {message}\n");
    let bytes = line.as_bytes();
    let _guard = LOG_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut wrote = false;
    let mut failures: Vec<String> = Vec::new();
    if let Some(path) = local_log_path() {
        match write_log_line(&path, bytes) {
            Ok(()) => wrote = true,
            Err(error) => failures.push(format!("LOCALAPPDATA {} ({error})", path.display())),
        }
    } else {
        failures.push("LOCALAPPDATA is unset".to_string());
    }
    if !wrote {
        if let Some(path) = exe_log_path() {
            let note = format!(
                "{millis} {message} | local log failed: {}\n",
                failures.join("; ")
            );
            match write_log_line(&path, note.as_bytes()) {
                Ok(()) => wrote = true,
                Err(error) => failures.push(format!("exe {} ({error})", path.display())),
            }
        }
    }
    if !wrote {
        if let Some(path) = temp_log_path() {
            let note = format!(
                "{millis} {message} | log sinks failed: {}\n",
                failures.join("; ")
            );
            let _ = write_log_line(&path, note.as_bytes());
        }
    }
}

fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // The panicking thread may already hold LOG_LOCK. Write without it.
        if LOGGING_PANIC.swap(true, Ordering::SeqCst) {
            previous(info);
            return;
        }
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
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_millis())
            .unwrap_or(0);
        let line = format!("{millis} PANIC at {location}: {message}\n");
        let bytes = line.as_bytes();
        let mut wrote = false;
        if let Some(path) = local_log_path() {
            wrote = write_log_line(&path, bytes).is_ok();
        }
        if !wrote {
            if let Some(path) = exe_log_path() {
                wrote = write_log_line(&path, bytes).is_ok();
            }
        }
        if !wrote {
            if let Some(path) = temp_log_path() {
                let _ = write_log_line(&path, bytes);
            }
        }
        LOGGING_PANIC.store(false, Ordering::SeqCst);
        previous(info);
    }));
}

static LOGGING_INIT: OnceLock<()> = OnceLock::new();
static LOGGING_PANIC: AtomicBool = AtomicBool::new(false);

/// First thing `main` calls, before `tauri::Builder`. Safe to call twice.
pub fn init_logging() {
    LOGGING_INIT.get_or_init(|| {
        install_panic_hook();
        log_line(&format!(
            "main enter pid={} cwd={:?} exe={:?} args={:?} LOCALAPPDATA={:?} TEMP={:?}",
            std::process::id(),
            std::env::current_dir().ok(),
            std::env::current_exe().ok(),
            std::env::args().collect::<Vec<_>>(),
            std::env::var("LOCALAPPDATA").ok(),
            std::env::var("TEMP").ok()
        ));
        log_line(&format!(
            "log sinks local={:?} exe={:?} temp={:?}",
            local_log_path(),
            exe_log_path(),
            temp_log_path()
        ));
        log_process_token();
    });
}

#[cfg(windows)]
fn log_process_token() {
    use winapi::um::errhandlingapi::GetLastError;
    use winapi::um::handleapi::CloseHandle;
    use winapi::um::processthreadsapi::{GetCurrentProcess, OpenProcessToken};
    use winapi::um::securitybaseapi::{
        GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation,
    };
    use winapi::um::winnt::{
        TokenElevation, TokenHasRestrictions, TokenIntegrityLevel, HANDLE,
        SECURITY_MANDATORY_HIGH_RID, SECURITY_MANDATORY_LOW_RID,
        SECURITY_MANDATORY_MEDIUM_PLUS_RID, SECURITY_MANDATORY_MEDIUM_RID,
        SECURITY_MANDATORY_SYSTEM_RID, SECURITY_MANDATORY_UNTRUSTED_RID, TOKEN_ELEVATION,
        TOKEN_MANDATORY_LABEL, TOKEN_QUERY,
    };

    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            log_line(&format!("OpenProcessToken failed os={}", GetLastError()));
            return;
        }
        let mut needed = 0u32;
        GetTokenInformation(
            token,
            TokenIntegrityLevel,
            std::ptr::null_mut(),
            0,
            &mut needed,
        );
        let mut integrity = "unknown".to_string();
        if needed > 0 && needed <= 4096 {
            let mut buffer = vec![0u8; needed as usize];
            if GetTokenInformation(
                token,
                TokenIntegrityLevel,
                buffer.as_mut_ptr() as *mut _,
                needed,
                &mut needed,
            ) != 0
            {
                let label = &*(buffer.as_ptr() as *const TOKEN_MANDATORY_LABEL);
                let sid = label.Label.Sid;
                if !sid.is_null() {
                    let count = *GetSidSubAuthorityCount(sid);
                    if count > 0 {
                        let rid = *GetSidSubAuthority(sid, (count - 1) as u32);
                        let name = if rid == SECURITY_MANDATORY_UNTRUSTED_RID {
                            "untrusted"
                        } else if rid == SECURITY_MANDATORY_LOW_RID {
                            "low"
                        } else if rid == SECURITY_MANDATORY_MEDIUM_RID {
                            "medium"
                        } else if rid == SECURITY_MANDATORY_MEDIUM_PLUS_RID {
                            "medium-plus"
                        } else if rid == SECURITY_MANDATORY_HIGH_RID {
                            "high"
                        } else if rid == SECURITY_MANDATORY_SYSTEM_RID {
                            "system"
                        } else {
                            "other"
                        };
                        integrity = format!("{name} rid={rid:#x}");
                    }
                }
            } else {
                integrity = format!("query failed os={}", GetLastError());
            }
        }
        let mut elevated = -1i32;
        let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut returned = 0u32;
        if GetTokenInformation(
            token,
            TokenElevation,
            &mut elevation as *mut _ as *mut _,
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned,
        ) != 0
        {
            elevated = elevation.TokenIsElevated as i32;
        }
        let mut restricted = -1i32;
        let mut flag = 0u32;
        if GetTokenInformation(
            token,
            TokenHasRestrictions,
            &mut flag as *mut _ as *mut _,
            std::mem::size_of::<u32>() as u32,
            &mut returned,
        ) != 0
        {
            restricted = if flag != 0 { 1 } else { 0 };
        }
        CloseHandle(token);
        log_line(&format!(
            "token integrity={integrity} elevated={elevated} restricted={restricted}"
        ));
    }
}

#[cfg(not(windows))]
fn log_process_token() {
    log_line("token integrity skipped");
}

static ERROR_BOX_SHOWN: AtomicBool = AtomicBool::new(false);

fn show_error_box(message: &str) {
    log_line(&format!("MessageBox: {message}"));
    #[cfg(windows)]
    {
        if ERROR_BOX_SHOWN.swap(true, Ordering::SeqCst) {
            return;
        }
        use std::os::windows::ffi::OsStrExt;
        use winapi::um::winuser::{MessageBoxW, MB_ICONERROR, MB_OK, MB_SETFOREGROUND, MB_TOPMOST};
        let mut text: Vec<u16> = std::ffi::OsStr::new(message)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let title: Vec<u16> = std::ffi::OsStr::new("AI Traffic Light")
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        unsafe {
            MessageBoxW(
                std::ptr::null_mut(),
                text.as_mut_ptr(),
                title.as_ptr(),
                MB_OK | MB_ICONERROR | MB_SETFOREGROUND | MB_TOPMOST,
            );
        }
    }
}

fn probe_writable(path: &std::path::Path) -> Result<(), String> {
    std::fs::create_dir_all(path).map_err(|error| format!("create_dir_all: {error}"))?;
    let probe = path.join(".write-probe");
    std::fs::write(&probe, b"ok").map_err(|error| format!("write: {error}"))?;
    let _ = std::fs::remove_file(&probe);
    Ok(())
}

fn webview_data_dir() -> Result<PathBuf, String> {
    static CHOSEN: OnceLock<Result<PathBuf, String>> = OnceLock::new();
    CHOSEN.get_or_init(choose_webview_data_dir).clone()
}

fn choose_webview_data_dir() -> Result<PathBuf, String> {
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        if !local.is_empty() {
            let default_dir = PathBuf::from(local).join("com.pcj1997.ai-traffic-light");
            match probe_writable(&default_dir) {
                Ok(()) => log_line(&format!(
                    "default WebView2 dir writable {}",
                    default_dir.display()
                )),
                Err(error) => log_line(&format!(
                    "default WebView2 dir not writable {} ({error})",
                    default_dir.display()
                )),
            }
        }
    }
    let mut candidates = Vec::new();
    if let Some(dir) = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|parent| parent.join("webview-data")))
    {
        candidates.push(dir);
    }
    if let Some(temp) = std::env::var_os("TEMP").or_else(|| std::env::var_os("TMP")) {
        if !temp.is_empty() {
            candidates.push(PathBuf::from(temp).join("ai-traffic-light-webview"));
        }
    }
    if let Some(profile) = std::env::var_os("USERPROFILE") {
        if !profile.is_empty() {
            candidates.push(
                PathBuf::from(profile)
                    .join("AppData")
                    .join("LocalLow")
                    .join("ai-traffic-light-webview"),
            );
        }
    }
    let mut failures = Vec::new();
    for path in candidates {
        match probe_writable(&path) {
            Ok(()) => {
                log_line(&format!("webview data directory {}", path.display()));
                return Ok(path);
            }
            Err(error) => {
                log_line(&format!(
                    "webview data dir rejected {} ({error})",
                    path.display()
                ));
                failures.push(format!("{} ({error})", path.display()));
            }
        }
    }
    Err(format!(
        "no writable WebView2 data directory: {}",
        failures.join("; ")
    ))
}

fn build_failure(stage: &str, error: &dyn std::fmt::Display) -> String {
    format!("{stage}: WebviewWindowBuilder::build failed: {error}")
}

fn confirm_webview(window: &tauri::WebviewWindow, stage: &str) -> Result<(), String> {
    #[cfg(windows)]
    match window.hwnd() {
        Ok(hwnd) => log_line(&format!("{stage} hwnd {:?}", hwnd.0)),
        Err(error) => {
            return Err(format!(
                "{stage}: hwnd unavailable immediately after build: {error}"
            ));
        }
    }
    match window.is_visible() {
        Ok(visible) => log_line(&format!("{stage} is_visible={visible}")),
        Err(error) => {
            return Err(format!(
                "{stage}: webview did not start (failed to receive message): {error}"
            ));
        }
    }
    Ok(())
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
    log_line("open_configured_window enter");
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
    let data_dir = webview_data_dir()?;
    log_line(&format!(
        "from_config build begin data_directory={}",
        data_dir.display()
    ));
    let built = tauri::WebviewWindowBuilder::from_config(app, &config)
        .map_err(|error| build_failure("from_config", &error))?
        .data_directory(data_dir)
        .visible(true)
        .build()
        .map_err(|error| build_failure("from_config", &error))?;
    log_line("from_config build returned Ok");
    if let Err(error) = confirm_webview(&built, "from_config") {
        log_line(&format!("from_config destroy broken window: {error}"));
        let _ = built.destroy();
        return Err(error);
    }
    Ok(built)
}

#[cfg(windows)]
fn log_top_level_windows(stage: &str) {
    use winapi::shared::minwindef::{BOOL, DWORD, LPARAM, TRUE};
    use winapi::shared::windef::{HWND, RECT};
    use winapi::um::processthreadsapi::GetCurrentProcessId;
    use winapi::um::winuser::{
        EnumWindows, GetClassNameW, GetWindowRect, GetWindowTextW, GetWindowThreadProcessId,
        IsWindowVisible,
    };

    struct Found(Mutex<Vec<String>>);
    static FOUND: OnceLock<Found> = OnceLock::new();
    let found = FOUND.get_or_init(|| Found(Mutex::new(Vec::new())));
    if let Ok(mut rows) = found.0.lock() {
        rows.clear();
    }
    let pid = unsafe { GetCurrentProcessId() };
    unsafe extern "system" fn visit(hwnd: HWND, pid: LPARAM) -> BOOL {
        let mut owner: DWORD = 0;
        unsafe { GetWindowThreadProcessId(hwnd, &mut owner) };
        if owner != pid as DWORD {
            return TRUE;
        }
        let mut class = [0u16; 128];
        let mut title = [0u16; 128];
        let class_len = unsafe { GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32) };
        let title_len = unsafe { GetWindowTextW(hwnd, title.as_mut_ptr(), title.len() as i32) };
        let class = String::from_utf16_lossy(&class[..class_len.max(0) as usize]);
        let title = String::from_utf16_lossy(&title[..title_len.max(0) as usize]);
        let mut rect = RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        unsafe { GetWindowRect(hwnd, &mut rect) };
        let visible = unsafe { IsWindowVisible(hwnd) != 0 };
        if let Some(Found(rows)) = FOUND.get() {
            if let Ok(mut rows) = rows.lock() {
                rows.push(format!(
                    "hwnd={:?} class={class:?} title={title:?} vis={visible} {},{} {}x{}",
                    hwnd,
                    rect.left,
                    rect.top,
                    rect.right - rect.left,
                    rect.bottom - rect.top
                ));
            }
        }
        TRUE
    }
    unsafe { EnumWindows(Some(visit), pid as LPARAM) };
    let rows = found.0.lock().map(|rows| rows.clone()).unwrap_or_default();
    if rows.is_empty() {
        log_line(&format!("windows[{stage}] pid={pid} (none)"));
    } else {
        for row in rows {
            log_line(&format!("windows[{stage}] {row}"));
        }
    }
}

#[cfg(not(windows))]
fn log_top_level_windows(stage: &str) {
    log_line(&format!("windows[{stage}] skipped"));
}

fn open_fallback_window(app: &AppHandle) -> Result<tauri::WebviewWindow, String> {
    let data_dir = webview_data_dir()?;
    log_line(&format!(
        "fallback build begin data_directory={}",
        data_dir.display()
    ));
    let built =
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
            .data_directory(data_dir)
            .build()
            .map_err(|error| build_failure("fallback", &error))?;
    log_line("fallback build returned Ok");
    if let Err(error) = confirm_webview(&built, "fallback") {
        log_line(&format!("fallback destroy broken window: {error}"));
        let _ = built.destroy();
        return Err(error);
    }
    Ok(built)
}

fn open_plain_window(app: &AppHandle) -> Result<tauri::WebviewWindow, String> {
    let data_dir = webview_data_dir()?;
    log_line(&format!(
        "plain build begin data_directory={}",
        data_dir.display()
    ));
    let built =
        tauri::WebviewWindowBuilder::new(app, "main", tauri::WebviewUrl::App("index.html".into()))
            .title("AI Traffic Light")
            .inner_size(START_LOGICAL_W, START_LOGICAL_H)
            .min_inner_size(MIN_LOGICAL_W, MIN_LOGICAL_H)
            .decorations(true)
            .transparent(false)
            .always_on_top(true)
            .resizable(true)
            .skip_taskbar(false)
            .visible(true)
            .focused(true)
            .data_directory(data_dir)
            .build()
            .map_err(|error| build_failure("plain", &error))?;
    log_line("plain build returned Ok");
    if let Err(error) = confirm_webview(&built, "plain") {
        log_line(&format!("plain destroy broken window: {error}"));
        let _ = built.destroy();
        return Err(error);
    }
    Ok(built)
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
            return Ok(());
        }
        Ok(Err(error)) => log_line(&format!("fallback window failed: {error}")),
        Err(_) => log_line("fallback window panicked"),
    }
    if app.get_webview_window("main").is_some() {
        log_line("main window appeared after transparent fallback");
        return Ok(());
    }
    let plain = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        open_plain_window(app).map(|_| ())
    }));
    match plain {
        Ok(Ok(())) => {
            log_line("plain window created");
            Ok(())
        }
        Ok(Err(error)) => {
            log_line(&format!("plain window failed: {error}"));
            show_error_box(&format!("主窗口创建失败。\n{error}"));
            Err(error)
        }
        Err(_) => {
            log_line("plain window panicked");
            let error = "plain window panicked".to_string();
            show_error_box(&format!("主窗口创建失败。\n{error}"));
            Err(error)
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
fn present_main_window(
    app: &AppHandle,
    place: bool,
    reset_to_start: bool,
    native: bool,
    focus: bool,
) {
    if PRESENCE_HIDDEN.load(Ordering::Relaxed) && !FORCE_VISIBLE.load(Ordering::Relaxed) {
        log_line("present skipped: no client in use");
        return;
    }
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
    let needs_focus = focus && (place || window.is_visible().ok() != Some(true));
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
                present_main_window(&for_task, place, false, true, true);
            })
            .is_err()
        {
            if PRESENCE_HIDDEN.load(Ordering::Relaxed) && !FORCE_VISIBLE.load(Ordering::Relaxed) {
                log_line("deferred present skipped: no client in use");
                return;
            }
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
            if PRESENCE_HIDDEN.load(Ordering::Relaxed) && !FORCE_VISIBLE.load(Ordering::Relaxed) {
                log_line("recreate kept hidden: no client in use");
                if let Some(window) = queued.get_webview_window("main") {
                    let _ = window.hide();
                }
                return;
            }
            present_main_window(&queued, true, true, true, true);
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

static STATUS_CACHE: OnceLock<Mutex<StatusSnapshot>> = OnceLock::new();

fn cache_status(snapshot: StatusSnapshot) {
    if let Ok(mut slot) = STATUS_CACHE
        .get_or_init(|| Mutex::new(StatusSnapshot::idle()))
        .lock()
    {
        *slot = snapshot;
    }
}

fn cached_status() -> StatusSnapshot {
    STATUS_CACHE
        .get()
        .and_then(|slot| slot.lock().ok().map(|snapshot| snapshot.clone()))
        .unwrap_or_else(StatusSnapshot::idle)
}

#[tauri::command]
fn get_status() -> StatusSnapshot {
    // The webview polls this every 200ms. Disk and sqlite stay on the reader thread.
    cached_status()
}

#[derive(serde::Serialize)]
struct PromptPage {
    prompts: Vec<String>,
    items: Vec<codex_local::UserPrompt>,
    note: String,
}

fn session_user_prompts_sync(client: String, id: String) -> Result<PromptPage, String> {
    if client != "codex" {
        return Ok(PromptPage {
            prompts: Vec::new(),
            items: Vec::new(),
            note: "该客户端暂不能读取提问原文，列表里仍可看到标题和状态。".to_string(),
        });
    }
    let items = codex_local::user_prompt_entries(&id)?;
    let note = if items.is_empty() {
        "这条会话里没有找到你的提问。"
    } else {
        "只显示你的提问，最新的在前。"
    };
    let prompts = items.iter().map(|item| item.text.clone()).collect();
    Ok(PromptPage {
        prompts,
        items,
        note: note.to_string(),
    })
}

#[tauri::command]
fn open_external_url(url: String) -> Result<(), String> {
    let url = url.trim();
    if url.len() > 2048
        || !(url.starts_with("https://") || url.starts_with("http://"))
        || url.chars().any(|ch| ch.is_control() || ch.is_whitespace())
    {
        return Err("只打开 http 或 https 链接".to_string());
    }
    spawn_open(url)
        .map(|_| ())
        .map_err(|err| format!("无法打开链接：{err}"))
}

#[cfg(windows)]
fn spawn_open(url: &str) -> std::io::Result<std::process::Child> {
    std::process::Command::new("rundll32")
        .args(["url.dll,FileProtocolHandler", url])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
}

#[cfg(not(windows))]
fn spawn_open(url: &str) -> std::io::Result<std::process::Child> {
    std::process::Command::new("xdg-open")
        .arg(url)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
}

#[tauri::command]
async fn session_user_prompts(client: String, id: String) -> Result<PromptPage, String> {
    tauri::async_runtime::spawn_blocking(move || session_user_prompts_sync(client, id))
        .await
        .map_err(|err| format!("读取提问失败：{err}"))?
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

#[derive(Debug, PartialEq, Eq)]
enum LampVisibility {
    Show,
    Hide,
    Leave,
}

/// Running or freshly stopped clients show the window. With none, hide it
/// unless the tray asked to keep it on screen. Repeating the same state does
/// not move a window the user can already see.
fn lamp_visibility(active: bool, forced: bool, window_hidden: bool) -> LampVisibility {
    if active {
        if window_hidden {
            LampVisibility::Show
        } else {
            LampVisibility::Leave
        }
    } else if forced || window_hidden {
        LampVisibility::Leave
    } else {
        LampVisibility::Hide
    }
}

fn apply_lamp_presence(app: &AppHandle, active: bool) {
    let forced = FORCE_VISIBLE.load(Ordering::Relaxed);
    let window_hidden = PRESENCE_HIDDEN.load(Ordering::Relaxed) || main_window_hidden(app);
    match lamp_visibility(active, forced, window_hidden) {
        LampVisibility::Show => {
            FORCE_VISIBLE.store(false, Ordering::Relaxed);
            PRESENCE_HIDDEN.store(false, Ordering::Relaxed);
            log_line("show: a client is in use");
            present_main_window(app, true, false, true, false);
        }
        LampVisibility::Hide => {
            PRESENCE_HIDDEN.store(true, Ordering::Relaxed);
            log_line("hide: no client in use");
            if let Some(window) = app.get_webview_window("main") {
                match window.hide() {
                    Ok(()) => log_line("hide ok"),
                    Err(error) => log_line(&format!("hide error: {error}")),
                }
            }
        }
        LampVisibility::Leave => {
            if active {
                FORCE_VISIBLE.store(false, Ordering::Relaxed);
                PRESENCE_HIDDEN.store(false, Ordering::Relaxed);
            } else if !forced {
                PRESENCE_HIDDEN.store(true, Ordering::Relaxed);
            }
        }
    }
}

#[tauri::command]
fn set_lamp_presence(app: AppHandle, active: bool) {
    let queued = app.clone();
    if app
        .run_on_main_thread(move || apply_lamp_presence(&queued, active))
        .is_err()
    {
        log_line("set_lamp_presence: main thread unavailable");
        apply_lamp_presence(&app, active);
    }
}

fn show_window(app: &AppHandle) {
    FORCE_VISIBLE.store(true, Ordering::Relaxed);
    PRESENCE_HIDDEN.store(false, Ordering::Relaxed);
    log_line("show from tray");
    let place = main_window_hidden(app) || launch_should_place();
    present_main_window(app, place, false, true, true);
}

// Page load is not required. If this never arrives (missing dist, custom
// protocol, or a blank webview), the launch timers still show the window.
#[tauri::command]
fn window_ready(app: AppHandle) {
    let queued = app.clone();
    if app
        .run_on_main_thread(move || {
            let place = main_window_hidden(&queued) || launch_should_place();
            present_main_window(&queued, place, false, true, true);
        })
        .is_err()
    {
        if PRESENCE_HIDDEN.load(Ordering::Relaxed) && !FORCE_VISIBLE.load(Ordering::Relaxed) {
            log_line("window_ready skipped: no client in use");
            return;
        }
        let _ = app.get_webview_window("main").map(|window| {
            let _ = window.unminimize();
            let _ = window.show();
            let _ = window.set_focus();
        });
    }
}

fn start_session_readers(app: AppHandle) {
    // Sqlite, rollout jsonl, powershell probes, and the notify config file all
    // stay off the UI thread, and they start only after the window exists.
    thread::spawn(move || {
        log_line("readers: starting off the UI thread");
        marvis::start();
        zcode::start();
        coze::start();
        trae::start();
        doubao::start();
        codex_local::start();
        workbuddy_local::start();
        reporter::start();
        notify::start(app);
        log_line("readers: worker threads spawned");
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    init_logging();
    log_line("run enter, before tauri::Builder");
    log_line("plugin register: single-instance");
    let builder = tauri::Builder::default()
        // Must be the first plugin: a second launch (shortcut re-click, duplicate
        // autostart entry) exits here and just raises the existing window instead
        // of stacking another lamp. Its setup calls process::exit on the second
        // process, which is why "setup begin" can be missing from that process.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            log_line("single-instance callback");
            FORCE_VISIBLE.store(true, Ordering::Relaxed);
            PRESENCE_HIDDEN.store(false, Ordering::Relaxed);
            let queued = app.clone();
            let _ = app.run_on_main_thread(move || {
                let place = main_window_hidden(&queued) || launch_should_place();
                present_main_window(&queued, place, false, true, true);
            });
        }));
    log_line("plugin register: notification");
    let builder = builder.plugin(tauri_plugin_notification::init());
    log_line("plugin register: autostart");
    let builder = builder
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .invoke_handler(tauri::generate_handler![
            get_status,
            session_user_prompts,
            open_external_url,
            resize_light_to_fit,
            set_lamp_presence,
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
            log_line("setup begin (plugins already finished)");
            log_top_level_windows("setup-begin");
            if std::env::args().any(|a| a == "--enable-autostart") {
                log_line("autostart enable begin");
                match app.autolaunch().enable() {
                    Ok(()) => log_line("autostart enabled"),
                    Err(error) => log_line(&format!("autostart error: {error}")),
                }
            }
            log_line("bridge::start begin");
            if let Err(error) = bridge::start() {
                log_line(&format!("bridge start error: {error}"));
                return Err(std::io::Error::other(error.to_string()).into());
            }
            log_line("bridge::start ok");

            // The config window is not auto-created (`create: false`). Build it
            // here so a failure is logged and the fallback builder can run,
            // instead of Tauri panicking before this hook. Session readers stay
            // off this thread until the window and tray exist.
            let _ = launch_clock();
            log_line("ensure_main_window begin");
            if let Err(error) = ensure_main_window(app.handle()) {
                log_line(&format!("setup ensure failed: {error}"));
            }
            log_line("ensure_main_window returned");
            log_top_level_windows("after-create");
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
            present_main_window(app.handle(), true, true, false, true);
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
            log_line("tray menu build");
            let notify_item = CheckMenuItemBuilder::with_id("notify-toggle", "会话完成通知")
                .checked(notify::enabled())
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
            log_line("tray icon build begin");
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
            log_line("tray icon build ok");

            let app_handle = app.handle().clone();
            let status_autostart_item = autostart_item.clone();
            let status_notify_item = notify_item.clone();
            thread::spawn(move || {
                log_line("status thread enter");
                let mut system = sysinfo::System::new();
                let mut logged_scan = false;
                loop {
                    // Presence still feeds the tray and team report. It must not hide
                    // the lamp: sessions are readable when the client process name
                    // does not match, and the window is meant to stay on screen.
                    let scan_started = Instant::now();
                    presence::refresh(&mut system);
                    if !logged_scan {
                        logged_scan = true;
                        log_line(&format!(
                            "status thread: process scan {}ms",
                            scan_started.elapsed().as_millis()
                        ));
                    }
                    thread::sleep(Duration::from_millis(500));
                    let snapshot = bridge::read();
                    cache_status(snapshot.clone());
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
                    let _ = status_notify_item.set_checked(notify::enabled());
                    if let Ok(enabled) = app_handle.autolaunch().is_enabled() {
                        let _ = status_autostart_item.set_checked(enabled);
                    }

                    // Keep the handle alive for the application's lifetime.
                    let _ = &app_handle;
                }
            });

            start_session_readers(app.handle().clone());
            log_top_level_windows("setup-end");
            log_line("setup end");
            Ok(())
        });
    log_line("builder.build begin");
    match builder.build(tauri::generate_context!()) {
        Ok(app) => {
            log_line("builder ok, entering event loop");
            app.run(|app, event| match event {
                tauri::RunEvent::Ready => {
                    log_line("RunEvent::Ready");
                    let place = main_window_hidden(app) || launch_should_place();
                    present_main_window(app, place, false, false, true);
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
        let wide = super::fit_logical_size(1100.0, 800.0).unwrap();
        assert_eq!(wide, (1100.0, 800.0));
        assert!(fitted.0 >= 72.0 && fitted.1 >= 120.0);
        assert!(super::fit_logical_size(f64::NAN, 400.0).is_none());
        let tiny = super::fit_logical_size(16.0, 16.0).unwrap();
        assert!(tiny.0 >= 72.0 && tiny.1 >= 120.0);
        let one_lamp = super::fit_logical_size(84.0, 160.0).unwrap();
        assert_eq!(one_lamp, (84.0, 160.0));
    }

    #[test]
    fn lamp_shows_only_while_a_client_is_in_use() {
        use super::LampVisibility::{Hide, Leave, Show};
        assert_eq!(super::lamp_visibility(true, false, true), Show);
        assert_eq!(super::lamp_visibility(true, true, false), Leave);
        assert_eq!(super::lamp_visibility(false, false, false), Hide);
        assert_eq!(super::lamp_visibility(false, true, false), Leave);
        assert_eq!(super::lamp_visibility(false, false, true), Leave);
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
