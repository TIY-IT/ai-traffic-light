//! 会话窗口聚焦:点击"会话已结束"系统通知或面板里的会话行时,
//! 把该会话所属的 AI 客户端窗口调到前台。
//! 匹配策略:桌面客户端优先按进程名,否则按窗口标题(cwd 目录名 / 客户端显示名)。

use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub struct WindowInfo {
    /// Win32 窗口句柄,以 isize 保存以便结构体可 Send。
    pub hwnd: isize,
    /// 进程镜像文件名(小写),如 "coze.exe"。
    pub process: String,
    /// 窗口标题(非空)。
    pub title: String,
}

/// 纯函数:按优先级挑选目标窗口。windows 顺序即 Z 序(EnumWindows 枚举顺序),
/// 多命中取第一个。进程名精确匹配优先;其次按标题 token 依次做包含匹配。
fn pick_window(
    windows: &[WindowInfo],
    process_hint: Option<&str>,
    title_tokens: &[String],
) -> Option<WindowInfo> {
    if let Some(process) = process_hint {
        if !process.is_empty() {
            if let Some(hit) = windows.iter().find(|w| w.process == process) {
                return Some(hit.clone());
            }
        }
    }
    for token in title_tokens {
        if token.is_empty() {
            continue;
        }
        let needle = token.to_lowercase();
        if let Some(hit) = windows
            .iter()
            .find(|w| w.title.to_lowercase().contains(&needle))
        {
            return Some(hit.clone());
        }
    }
    None
}

/// 桌面应用客户端按进程名直接定位;CLI 类(跑在终端里)返回 None 走标题匹配。
fn process_hint(client: &str) -> Option<String> {
    let exe = match client {
        "coze" => "coze.exe",
        "doubao" => "doubao.exe",
        "trae" => "trae.exe",
        "marvis" => "marvis.exe",
        "workbuddy" => "workbuddy.exe",
        "codex" => "codex.exe",
        _ => return None,
    };
    Some(exe.to_string())
}

/// 标题匹配 token:依次尝试 cwd 的目录名(非空时)、客户端显示名。
fn title_tokens(client: &str, cwd_basename: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    if !cwd_basename.is_empty() {
        tokens.push(cwd_basename.to_string());
    }
    let display = match client {
        "zcode" => "ZCode",
        "codex" => "Codex",
        "claude" => "Claude",
        "codebuddy" => "CodeBuddy",
        other => other,
    };
    tokens.push(display.to_string());
    tokens
}

/// 读会话文件里的 cwd 字段。读不到(文件缺失/解析失败/id 非法)返回 None。
/// 与 sessions.rs 一样校验 id 字符集,防止借路径逃逸出 sessions 目录。
fn session_cwd(id: &str) -> Option<String> {
    let valid = !id.is_empty()
        && id != "."
        && id != ".."
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c));
    if !valid {
        return None;
    }
    let path = dirs::home_dir()?
        .join(".ai-traffic-light")
        .join("sessions")
        .join(format!("{id}.json"));
    let content = std::fs::read_to_string(path).ok()?;
    let root: Value = serde_json::from_str(content.trim_start_matches('\u{feff}')).ok()?;
    root.get("cwd").and_then(|v| v.as_str()).map(str::to_string)
}

#[cfg(windows)]
mod win32 {
    use super::WindowInfo;
    use winapi::shared::minwindef::{DWORD, LPARAM, TRUE};
    use winapi::shared::windef::HWND;
    use winapi::um::winuser;

    unsafe extern "system" fn collect_hwnds(hwnd: HWND, lparam: LPARAM) -> i32 {
        let sink = &mut *(lparam as *mut Vec<isize>);
        sink.push(hwnd as isize);
        TRUE
    }

    /// 枚举可见顶层窗口:跳过空标题和本进程窗口;进程名统一小写。
    /// 顺序即 Z 序,交给 pick_window 取首个命中。
    pub fn enumerate_visible_windows() -> Vec<WindowInfo> {
        let mut hwnds: Vec<isize> = Vec::new();
        unsafe {
            winuser::EnumWindows(
                Some(collect_hwnds),
                &mut hwnds as *mut Vec<isize> as LPARAM,
            );
        }

        let mut candidates: Vec<(isize, u32, String)> = Vec::new();
        let self_pid = std::process::id();
        for hwnd in hwnds {
            let handle = hwnd as HWND;
            let (visible, title, pid) = unsafe {
                (
                    winuser::IsWindowVisible(handle) != 0,
                    window_title(handle),
                    {
                        let mut pid: DWORD = 0;
                        winuser::GetWindowThreadProcessId(handle, &mut pid);
                        pid
                    },
                )
            };
            if !visible || title.is_empty() || pid == self_pid || pid == 0 {
                continue;
            }
            candidates.push((hwnd, pid, title));
        }

        // 用 sysinfo(项目已有依赖)把 pid 映射成镜像文件名,避免为 winapi 追加 psapi/winbase feature。
        let pids: Vec<sysinfo::Pid> = candidates
            .iter()
            .map(|(_, pid, _)| sysinfo::Pid::from_u32(*pid))
            .collect();
        let mut system = sysinfo::System::new();
        system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&pids), true);

        candidates
            .into_iter()
            .map(|(hwnd, pid, title)| {
                let process = system
                    .process(sysinfo::Pid::from_u32(pid))
                    .map(|p| p.name().to_string_lossy().to_lowercase())
                    .unwrap_or_default();
                WindowInfo { hwnd, process, title }
            })
            .collect()
    }

    unsafe fn window_title(hwnd: HWND) -> String {
        let len = winuser::GetWindowTextLengthW(hwnd);
        if len <= 0 {
            return String::new();
        }
        let mut buf = vec![0u16; (len + 1) as usize];
        let copied = winuser::GetWindowTextW(hwnd, buf.as_mut_ptr(), len + 1);
        if copied <= 0 {
            return String::new();
        }
        String::from_utf16_lossy(&buf[..copied as usize])
    }

    /// 先 SwitchToThisWindow 再 SetForegroundWindow 兜底。
    fn bring_to_front(hwnd: isize) {
        let handle = hwnd as HWND;
        unsafe {
            winuser::SwitchToThisWindow(handle, TRUE);
            winuser::SetForegroundWindow(handle);
        }
    }

    pub fn run(client: &str, id: &str) -> Result<(), String> {
        let cwd = super::session_cwd(id).unwrap_or_default();
        let cwd_basename = std::path::Path::new(&cwd)
            .file_name()
            .and_then(|v| v.to_str())
            .map(str::to_string)
            .unwrap_or_default();
        let hint = super::process_hint(client);
        let tokens = super::title_tokens(client, &cwd_basename);
        match super::pick_window(&enumerate_visible_windows(), hint.as_deref(), &tokens) {
            Some(hit) => {
                bring_to_front(hit.hwnd);
                Ok(())
            }
            None => Err("未找到对应客户端窗口".to_string()),
        }
    }
}

pub fn focus_session(client: &str, id: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        win32::run(client, id)
    }
    #[cfg(not(windows))]
    {
        let _ = (client, id);
        Err("当前平台不支持窗口聚焦".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(hwnd: isize, process: &str, title: &str) -> WindowInfo {
        WindowInfo {
            hwnd,
            process: process.to_string(),
            title: title.to_string(),
        }
    }

    #[test]
    fn process_name_match_wins_over_title() {
        // 进程名精确命中;即便标题 token 也命中别的窗口,进程名优先。
        let windows = vec![
            window(1, "explorer.exe", "ai-traffic-light"),
            window(2, "coze.exe", "扣子"),
        ];
        let tokens = vec!["扣子".to_string()];
        let picked = pick_window(&windows, Some("coze.exe"), &tokens).unwrap();
        assert_eq!(picked.hwnd, 2);
    }

    #[test]
    fn title_token_match_is_case_insensitive_in_z_order() {
        // 无进程名提示(CLI 类)时按标题 token 匹配;多命中取 Z 序第一个,大小写不敏感。
        let windows = vec![
            window(1, "windowsterminal.exe", "ZCode — D:\\3Dauto\\demo"),
            window(2, "windowsterminal.exe", "zcode — D:\\play"),
        ];
        let tokens = vec!["demo".to_string(), "ZCode".to_string()];
        let picked = pick_window(&windows, None, &tokens).unwrap();
        assert_eq!(picked.hwnd, 1);

        // 第二个 token 也能兜底命中。
        let picked = pick_window(&windows, None, &["play".to_string()]).unwrap();
        assert_eq!(picked.hwnd, 2);
    }

    #[test]
    fn no_match_returns_none() {
        let windows = vec![
            window(1, "explorer.exe", "文件资源管理器"),
            window(2, "code.exe", "Visual Studio Code"),
        ];
        assert!(pick_window(&windows, Some("coze.exe"), &["ai-traffic-light".to_string()]).is_none());
        assert!(pick_window(&windows, None, &["不存在的项目".to_string()]).is_none());
        // 空 token 不应造成误命中。
        assert!(pick_window(&windows, None, &[String::new()]).is_none());
    }
}
