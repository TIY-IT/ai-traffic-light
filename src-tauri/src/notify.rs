//! 会话完成系统通知:轮询聚合快照,某会话"见过运行"后转入结束态时弹一条系统 toast。
use crate::sessions::SessionSnapshot;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::AppHandle;

static ENABLED: AtomicBool = AtomicBool::new(true);

/// 同一会话两次结束通知的最小间隔,防止反复运行/结束的会话刷屏。
const NOTIFY_COOLDOWN: Duration = Duration::from_secs(90);

/// 纯函数:距上次通知已满冷却期(或从未通知过)才允许再次发送。
fn cooldown_ok(
    last: Option<std::time::Instant>,
    now: std::time::Instant,
    cooldown: std::time::Duration,
) -> bool {
    match last {
        Some(at) => now.duration_since(at) >= cooldown,
        None => true,
    }
}

fn config_path() -> Option<PathBuf> {
    Some(dirs::home_dir()?.join(".ai-traffic-light").join("config.json"))
}

/// 顶层 notify_enabled 键;文件/键缺失时视为开启(托盘默认勾选)。
pub fn load_notify_enabled() -> bool {
    let Some(path) = config_path() else { return true };
    let Ok(content) = std::fs::read_to_string(&path) else { return true };
    let Ok(root) = serde_json::from_str::<serde_json::Value>(content.trim_start_matches('\u{feff}')) else {
        return true;
    };
    root.get("notify_enabled").and_then(|v| v.as_bool()).unwrap_or(true)
}

/// 写顶层 notify_enabled,整体读改写以保留文件里其他键(reporter 的 team 也存这里)。
pub fn save_notify_enabled(enabled: bool) -> Result<(), String> {
    let path = config_path().ok_or_else(|| "无法定位用户目录".to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let mut root: serde_json::Value = match std::fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(content.trim_start_matches('\u{feff}'))
            .unwrap_or_else(|_| serde_json::json!({})),
        Err(_) => serde_json::json!({}),
    };
    root["notify_enabled"] = serde_json::Value::Bool(enabled);
    let pretty = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
    std::fs::write(&path, pretty).map_err(|e| e.to_string())
}

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

pub fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::Relaxed);
    let _ = save_notify_enabled(enabled);
}

fn client_display(client: &str) -> &'static str {
    match client {
        "zcode" => "ZCode",
        "coze" => "扣子",
        "trae" => "TRAE",
        "doubao" => "豆包",
        "codex" => "Codex",
        "workbuddy" => "WorkBuddy",
        "marvis" => "Marvis",
        "claude" => "Claude",
        _ => "CodeBuddy",
    }
}

fn body_text(session: &SessionSnapshot) -> String {
    let base = if session.title.is_empty() { session.label.as_str() } else { session.title.as_str() };
    base.chars().take(120).collect()
}

/// 纯函数:返回本拍应通知的会话 id。仅"见过运行"(seen)且上一帧 working、
/// 当前帧转入 completed/waiting/error 的才通知;通知方负责随后把 id 移出 seen。
fn transitions(
    prev: &HashMap<String, String>,
    next: &[SessionSnapshot],
    seen: &HashSet<String>,
) -> Vec<String> {
    next.iter()
        .filter(|s| {
            seen.contains(&s.id)
                && prev.get(&s.id).map(String::as_str) == Some("working")
                && ["completed", "waiting", "error"].contains(&s.state.as_str())
        })
        .map(|s| s.id.clone())
        .collect()
}

fn send(app: &AppHandle, session: &SessionSnapshot) {
    // 直接用 tauri-winrt-notification(插件的 Windows 后端)以支持点击回调:
    // 插件的 Rust builder 不支持 on_activated。app_id 必须与插件一致(identifier),
    // 保证复用已注册的 AUMID 显示名。
    let app_id = app.config().identifier.clone();
    let client2 = session.client.clone();
    let id2 = session.id.clone();
    let title = format!("AI 状态灯 · {}会话已结束", client_display(&session.client));
    let _ = tauri_winrt_notification::Toast::new(&app_id)
        .title(&title)
        .text1(&body_text(session))
        .on_activated(move |_| {
            let _ = crate::focus::focus_session(&client2, &id2);
            Ok(())
        })
        .show();
}

/// 每 500ms 比对一帧;开关关闭时只跟进状态基准、不发通知,避免重新开启后
/// 为关闭期间早已结束的会话补发轰炸。
pub fn start(app: AppHandle) {
    std::thread::spawn(move || {
        // Config lives under the user profile. A redirected home directory must
        // not block the UI thread that creates the window.
        ENABLED.store(load_notify_enabled(), Ordering::Relaxed);
        let mut prev: HashMap<String, String> = HashMap::new();
        let mut seen: HashSet<String> = HashSet::new();
        let mut last_notified: HashMap<String, std::time::Instant> = HashMap::new();
        loop {
            std::thread::sleep(Duration::from_millis(500));
            let now = std::time::Instant::now();
            let snapshot = crate::bridge::read();
            if enabled() {
                let ids = transitions(&prev, &snapshot.sessions, &seen);
                if ids.len() > 5 {
                    // 防洪水:单拍超 5 条视为冷启动/状态重置,只发第一条并清空"见过运行"。
                    seen.clear();
                    if let Some(id) = ids.first() {
                        if cooldown_ok(last_notified.get(id).copied(), now, NOTIFY_COOLDOWN) {
                            if let Some(session) = snapshot.sessions.iter().find(|s| &s.id == id) {
                                last_notified.insert(id.clone(), now);
                                send(&app, session);
                            }
                        }
                    }
                } else {
                    for id in ids {
                        if let Some(session) = snapshot.sessions.iter().find(|s| s.id == id) {
                            // 同一次结束只通知一次:通知后移出 seen,再次运行会重新记入。
                            // 冷却期内跳过发送但同样移出 seen,防重逻辑不受影响。
                            seen.remove(&session.id);
                            if cooldown_ok(
                                last_notified.get(&session.id).copied(),
                                now,
                                NOTIFY_COOLDOWN,
                            ) {
                                last_notified.insert(session.id.clone(), now);
                                send(&app, &session);
                            }
                        }
                    }
                }
            }
            // 见过运行集合与上一帧每拍都更新(关通知时也更新),保证比对基准始终最新。
            for session in &snapshot.sessions {
                if session.state == "working" {
                    seen.insert(session.id.clone());
                }
            }
            prev = snapshot
                .sessions
                .iter()
                .map(|s| (s.id.clone(), s.state.clone()))
                .collect();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session(id: &str, state: &str) -> SessionSnapshot {
        SessionSnapshot {
            id: id.into(),
            client: "zcode".into(),
            title: format!("title-{id}"),
            state: state.into(),
            label: "已结束".into(),
            message: String::new(),
            updated_at: 0,
            created_at: 0,
        }
    }

    fn prev_of(entries: &[(&str, &str)]) -> HashMap<String, String> {
        entries
            .iter()
            .map(|(id, state)| (id.to_string(), state.to_string()))
            .collect()
    }

    fn seen_of(ids: &[&str]) -> HashSet<String> {
        ids.iter().map(|id| id.to_string()).collect()
    }

    #[test]
    fn working_to_completed_notifies() {
        let prev = prev_of(&[("a", "working")]);
        let seen = seen_of(&["a"]);
        let next = vec![session("a", "completed")];
        assert_eq!(transitions(&prev, &next, &seen), vec!["a".to_string()]);
    }

    #[test]
    fn waiting_and_error_endings_also_notify() {
        let prev = prev_of(&[("a", "working"), ("b", "working")]);
        let seen = seen_of(&["a", "b"]);
        let next = vec![session("a", "waiting"), session("b", "error")];
        assert_eq!(
            transitions(&prev, &next, &seen),
            vec!["a".to_string(), "b".to_string()]
        );
    }

    #[test]
    fn never_seen_running_is_silent() {
        // 直接以结束态出现(未见运行)的会话不通知。
        let prev = prev_of(&[("a", "working")]);
        let next = vec![session("a", "completed")];
        assert!(transitions(&prev, &next, &HashSet::new()).is_empty());
        let next = vec![session("b", "completed")];
        assert!(transitions(&HashMap::new(), &next, &HashSet::new()).is_empty());
    }

    #[test]
    fn completed_to_idle_is_silent() {
        let prev = prev_of(&[("a", "completed")]);
        let seen = seen_of(&["a"]);
        let next = vec![session("a", "idle")];
        assert!(transitions(&prev, &next, &seen).is_empty());
    }

    #[test]
    fn same_ending_notifies_once() {
        // 通知后 id 已移出 seen:同样的 working→completed 再出现不重复通知。
        let prev = prev_of(&[("a", "working")]);
        let next = vec![session("a", "completed")];
        assert!(transitions(&prev, &next, &HashSet::new()).is_empty());
    }

    #[test]
    fn cooldown_blocks_within_window() {
        // 90 秒冷却期内(含恰好差 1 秒)不允许再次发送。
        let last = std::time::Instant::now();
        assert!(!cooldown_ok(
            Some(last),
            last + Duration::from_secs(89),
            NOTIFY_COOLDOWN
        ));
    }

    #[test]
    fn cooldown_allows_after_window() {
        // 满 90 秒即放行;从未通知过的(None)始终放行。
        let last = std::time::Instant::now();
        assert!(cooldown_ok(
            Some(last),
            last + Duration::from_secs(90),
            NOTIFY_COOLDOWN
        ));
        assert!(cooldown_ok(
            Some(last),
            last + Duration::from_secs(600),
            NOTIFY_COOLDOWN
        ));
        assert!(cooldown_ok(None, last, NOTIFY_COOLDOWN));
    }
}
