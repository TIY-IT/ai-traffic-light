use std::sync::Mutex;
use sysinfo::{System, ProcessesToUpdate};
static CLIENTS: Mutex<Vec<String>> = Mutex::new(Vec::new());
fn client(name: &str) -> Option<&'static str> {
    // macOS process names carry no .exe suffix; match both forms.
    let lower = name.to_ascii_lowercase();
    let base = lower.strip_suffix(".exe").unwrap_or(&lower);
    match base {
        "codex" => Some("codex"),
        "workbuddy" => Some("workbuddy"),
        "marvis" => Some("marvis"),
        "zcode" => Some("zcode"),
        "coze" => Some("coze"),
        "doubao" => Some("doubao"),
        "trae solo cn" | "trae cn" | "trae" => Some("trae"),
        _ => None,
    }
}
pub fn refresh(system: &mut System) -> Vec<String> {
    system.refresh_processes(ProcessesToUpdate::All, true);
    let mut clients: Vec<String> = system.processes().values()
        .filter_map(|p| client(&p.name().to_string_lossy()).map(str::to_string)).collect();
    clients.sort(); clients.dedup();
    if let Ok(mut saved) = CLIENTS.lock() { *saved = clients.clone(); }
    clients
}
#[tauri::command]
pub fn get_active_clients() -> Vec<String> { CLIENTS.lock().map(|v| v.clone()).unwrap_or_default() }
#[cfg(test)] mod tests {
    use super::*;
    #[test] fn helpers_do_not_count_as_apps() {
        assert_eq!(client("MarvisSvr.exe"), None);
        assert_eq!(client("MarvisHost.exe"), None);
        assert_eq!(client("codex-code-mode-host.exe"), None);
        assert_eq!(client("Marvis.exe"), Some("marvis"));
        assert_eq!(client("WorkBuddy.exe"), Some("workbuddy"));
        assert_eq!(client("Codex.exe"), Some("codex"));
    }

    #[test] fn macos_process_names_have_no_exe_suffix() {
        assert_eq!(client("Codex"), Some("codex"));
        assert_eq!(client("WorkBuddy"), Some("workbuddy"));
        assert_eq!(client("Coze"), Some("coze"));
        assert_eq!(client("ZCode"), Some("zcode"));
        assert_eq!(client("MarvisSvr"), None);
        assert_eq!(client("codex-code-mode-host"), None);
    }

    #[test] fn doubao_and_trae_process_names_map_to_clients() {
        assert_eq!(client("Doubao.exe"), Some("doubao"));
        assert_eq!(client("TRAE SOLO CN.exe"), Some("trae"));
        assert_eq!(client("Trae.exe"), Some("trae"));
        assert_eq!(client("TRAE SOLO CNHelper.exe"), None);
        assert_eq!(client("Doubao"), Some("doubao"));
    }
}
