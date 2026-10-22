//! Plain-Chinese session summaries. The DeepSeek key is never written to the log.
use crate::codex_local::{conversation_excerpt, UserPrompt};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

pub const SUMMARY_CHAR_LIMIT: usize = 4000;
const ASSISTANT_TAIL_CHARS: usize = 1600;

const SYSTEM_PROMPT: &str = "\
你是会话进度摘要器。根据用户提问和助手回复的结尾，用简体中文写正好三行，每行一句短话。不要 markdown，不要项目符号，不要额外解释。格式必须是：\n\
在做什么：…\n\
做到哪一步：…\n\
还需要你做什么：…\n\
信息不够时，对应行写“暂不清楚”。";

#[derive(Clone, Debug, Serialize)]
pub struct SummaryPayload {
    /// `ready`, `pending`, `empty`, or `unavailable`.
    pub state: String,
    pub text: String,
}

#[derive(Default, Serialize, Deserialize)]
struct SummaryCacheFile {
    sessions: BTreeMap<String, SummaryCacheEntry>,
}

#[derive(Clone, Serialize, Deserialize)]
struct SummaryCacheEntry {
    key: String,
    text: String,
}

struct Memory {
    /// session id -> (cache key currently being generated)
    running: HashMap<String, String>,
    /// session id -> cache key that failed recently
    failed: HashMap<String, (String, Instant)>,
    /// session id -> (cache key, summary) when the exe directory is not writable
    memory: HashMap<String, (String, String)>,
}

fn memory() -> &'static Mutex<Memory> {
    static MEMORY: LazyLock<Mutex<Memory>> = LazyLock::new(|| {
        Mutex::new(Memory {
            running: HashMap::new(),
            failed: HashMap::new(),
            memory: HashMap::new(),
        })
    });
    &MEMORY
}

fn lock_memory() -> std::sync::MutexGuard<'static, Memory> {
    memory().lock().unwrap_or_else(|err| err.into_inner())
}

/// Stable identity of the latest prompt. Earlier prompts do not affect the key.
pub fn summary_cache_key(prompts: &[UserPrompt]) -> Option<String> {
    let last = prompts.last()?;
    Some(fnv_hex(&format!("{}\n{}", last.at, last.text)))
}

fn fnv_hex(text: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for byte in text.as_bytes() {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("{hash:016x}")
}

pub fn build_summary_input(prompts: &[String], assistant_tail: &str) -> String {
    let tail = tail_chars(assistant_tail.trim(), ASSISTANT_TAIL_CHARS);
    let lines: Vec<String> = prompts
        .iter()
        .map(|prompt| one_line(prompt))
        .filter(|prompt| !prompt.is_empty())
        .collect();
    let mut start = 0;
    loop {
        let text = render_summary_input(&lines[start..], &tail);
        if text.chars().count() <= SUMMARY_CHAR_LIMIT || start + 1 >= lines.len() {
            return tail_chars(&text, SUMMARY_CHAR_LIMIT);
        }
        start += 1;
    }
}

fn render_summary_input(prompts: &[String], tail: &str) -> String {
    let mut out = String::from("用户提问:\n");
    if prompts.is_empty() {
        out.push_str("（没有提问）\n");
    } else {
        for (index, prompt) in prompts.iter().enumerate() {
            out.push_str(&format!("{}. {}\n", index + 1, prompt));
        }
    }
    if !tail.is_empty() {
        out.push_str("助手最后回复的结尾:\n");
        out.push_str(tail);
        out.push('\n');
    }
    out
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn tail_chars(text: &str, max_chars: usize) -> String {
    let count = text.chars().count();
    if count <= max_chars {
        return text.to_string();
    }
    text.chars().skip(count - max_chars).collect()
}

pub fn clamp_summary(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(3)
        .collect::<Vec<_>>()
        .join("\n")
}

/// `%APPDATA%\ai-traffic-light\deepseek_api_key`, then `<exe dir>\deepseek_api_key`.
pub fn api_key_files() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Some(appdata) = std::env::var_os("APPDATA") {
        if !appdata.is_empty() {
            paths.push(
                PathBuf::from(appdata)
                    .join("ai-traffic-light")
                    .join("deepseek_api_key"),
            );
        }
    }
    if let Some(dir) = exe_dir() {
        paths.push(dir.join("deepseek_api_key"));
    }
    paths
}

pub fn cache_file_path() -> Option<PathBuf> {
    Some(exe_dir()?.join("summary-cache.json"))
}

fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
}

pub fn load_api_key() -> Option<String> {
    let env_key = std::env::var("DEEPSEEK_API_KEY")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    load_api_key_from(env_key.as_deref(), &api_key_files())
}

pub fn load_api_key_from(env_value: Option<&str>, files: &[PathBuf]) -> Option<String> {
    if let Some(value) = env_value.map(str::trim).filter(|value| !value.is_empty()) {
        return Some(value.to_string());
    }
    for path in files {
        if let Some(value) = read_key_file(path) {
            return Some(value);
        }
    }
    None
}

fn read_key_file(path: &Path) -> Option<String> {
    let text = fs::read_to_string(path).ok()?;
    let line = text.trim_start_matches('\u{feff}').lines().next()?.trim();
    if line.is_empty() {
        None
    } else {
        Some(line.to_string())
    }
}

pub fn read_cached_summary(path: &Path, session_id: &str, key: &str) -> Option<String> {
    let file = read_cache_file(path)?;
    let entry = file.sessions.get(session_id)?;
    if entry.key == key && !entry.text.is_empty() {
        Some(entry.text.clone())
    } else {
        None
    }
}

pub fn write_cached_summary(
    path: &Path,
    session_id: &str,
    key: &str,
    text: &str,
) -> Result<(), String> {
    let mut file = read_cache_file(path).unwrap_or_default();
    file.sessions.insert(
        session_id.to_string(),
        SummaryCacheEntry {
            key: key.to_string(),
            text: text.to_string(),
        },
    );
    if let Some(parent) = path.parent() {
        if parent.components().next().is_some() {
            fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
    }
    let bytes = serde_json::to_vec_pretty(&file).map_err(|err| err.to_string())?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, &bytes).map_err(|err| err.to_string())?;
    if fs::rename(&tmp, path).is_err() {
        fs::write(path, &bytes).map_err(|err| err.to_string())?;
        let _ = fs::remove_file(&tmp);
    }
    Ok(())
}

fn read_cache_file(path: &Path) -> Option<SummaryCacheFile> {
    let bytes = fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn model_name() -> String {
    std::env::var("DEEPSEEK_MODEL")
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "deepseek-chat".to_string())
}

fn api_url() -> String {
    let base = std::env::var("DEEPSEEK_BASE_URL")
        .ok()
        .map(|value| value.trim().trim_end_matches('/').to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "https://api.deepseek.com".to_string());
    format!("{base}/chat/completions")
}

fn request_summary(api_key: &str, input: &str) -> Result<String, String> {
    let body = serde_json::json!({
        "model": model_name(),
        "temperature": 0.2,
        "messages": [
            {"role": "system", "content": SYSTEM_PROMPT},
            {"role": "user", "content": input}
        ]
    });
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(25))
        .build()
        .map_err(|_| "无法连接摘要服务".to_string())?;
    let response = client
        .post(api_url())
        .bearer_auth(api_key)
        .json(&body)
        .send()
        .map_err(|_| "摘要服务连不上".to_string())?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("摘要服务返回 {status}"));
    }
    let payload: serde_json::Value = response
        .json()
        .map_err(|_| "摘要响应无法解析".to_string())?;
    let text = payload
        .pointer("/choices/0/message/content")
        .and_then(|value| value.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let clamped = clamp_summary(&text);
    if clamped.is_empty() {
        Err("摘要是空的".to_string())
    } else {
        Ok(clamped)
    }
}

fn remember(session_id: &str, key: &str, text: &str) {
    if let Some(path) = cache_file_path() {
        if write_cached_summary(&path, session_id, key, text).is_err() {
            crate::log_line("summary: cache file was not written");
        }
    }
    lock_memory()
        .memory
        .insert(session_id.to_string(), (key.to_string(), text.to_string()));
}

fn cached_text(session_id: &str, key: &str) -> Option<String> {
    if let Some((cached_key, text)) = lock_memory().memory.get(session_id) {
        if cached_key == key && !text.is_empty() {
            return Some(text.clone());
        }
    }
    let path = cache_file_path()?;
    read_cached_summary(&path, session_id, key)
}

fn spawn_generation(session_id: String, key: String, input: String, api_key: String) {
    std::thread::spawn(move || {
        match request_summary(&api_key, &input) {
            Ok(text) => {
                remember(&session_id, &key, &text);
                let mut gate = lock_memory();
                gate.failed.remove(&session_id);
            }
            Err(error) => {
                // `error` is a status or a fixed phrase. It does not include the key.
                crate::log_line(&format!("summary: request failed {error}"));
                lock_memory()
                    .failed
                    .insert(session_id.clone(), (key.clone(), Instant::now()));
            }
        }
        lock_memory().running.remove(&session_id);
    });
}

pub fn session_summary_sync(client: &str, id: &str) -> SummaryPayload {
    if client != "codex" {
        return unavailable();
    }
    let (prompts, assistant) = match conversation_excerpt(id) {
        Ok(excerpt) => excerpt,
        Err(error) => {
            crate::log_line(&format!("summary: read failed {error}"));
            return unavailable();
        }
    };
    let Some(key) = summary_cache_key(&prompts) else {
        return SummaryPayload {
            state: "empty".to_string(),
            text: String::new(),
        };
    };
    if let Some(text) = cached_text(id, &key) {
        return SummaryPayload {
            state: "ready".to_string(),
            text,
        };
    }
    {
        let gate = lock_memory();
        if let Some((failed_key, at)) = gate.failed.get(id) {
            if failed_key == &key && at.elapsed() < Duration::from_secs(120) {
                return unavailable();
            }
        }
        if gate.running.contains_key(id) {
            return pending();
        }
    }
    let Some(api_key) = load_api_key() else {
        return unavailable();
    };
    let input = build_summary_input(
        &prompts
            .iter()
            .map(|prompt| prompt.text.clone())
            .collect::<Vec<_>>(),
        &assistant,
    );
    {
        let mut gate = lock_memory();
        if gate.running.contains_key(id) {
            return pending();
        }
        gate.running.insert(id.to_string(), key.clone());
        gate.failed.remove(id);
    }
    spawn_generation(id.to_string(), key, input, api_key);
    pending()
}

fn pending() -> SummaryPayload {
    SummaryPayload {
        state: "pending".to_string(),
        text: String::new(),
    }
}

fn unavailable() -> SummaryPayload {
    SummaryPayload {
        state: "unavailable".to_string(),
        text: String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prompt(text: &str, at: i64) -> UserPrompt {
        UserPrompt {
            text: text.to_string(),
            at,
        }
    }

    #[test]
    fn summary_cache_key_uses_only_the_last_prompt() {
        let first = summary_cache_key(&[prompt("改首页", 10), prompt("改导航", 20)]).unwrap();
        let same_last =
            summary_cache_key(&[prompt("完全不同的旧提问", 1), prompt("改导航", 20)]).unwrap();
        let changed_text =
            summary_cache_key(&[prompt("改首页", 10), prompt("改配色", 20)]).unwrap();
        let changed_time =
            summary_cache_key(&[prompt("改首页", 10), prompt("改导航", 21)]).unwrap();
        assert_eq!(first, same_last);
        assert_ne!(first, changed_text);
        assert_ne!(first, changed_time);
        assert!(summary_cache_key(&[]).is_none());

        let dir = std::env::temp_dir().join(format!(
            "summary-cache-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("summary-cache.json");
        let text = "在做什么：改导航\n做到哪一步：已改标题\n还需要你做什么：确认颜色";
        write_cached_summary(&path, "codex:abc", &first, text).unwrap();
        write_cached_summary(&path, "codex:other", &changed_text, "在做什么：另一条").unwrap();
        assert_eq!(
            read_cached_summary(&path, "codex:abc", &first).as_deref(),
            Some(text)
        );
        assert!(read_cached_summary(&path, "codex:abc", &changed_text).is_none());
        assert_eq!(
            read_cached_summary(&path, "codex:other", &changed_text).as_deref(),
            Some("在做什么：另一条")
        );
        assert_eq!(
            read_cached_summary(&path, "codex:abc", &first).as_deref(),
            Some(text)
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn summary_input_keeps_latest_prompts_and_assistant_tail() {
        let prompts = vec!["帮我改首页".to_string(), "再改导航".to_string()];
        let input = build_summary_input(&prompts, "我已经改了标题，还差配色");
        assert!(input.contains("帮我改首页"));
        assert!(input.contains("再改导航"));
        assert!(input.contains("助手最后回复的结尾:"));
        assert!(input.contains("我已经改了标题，还差配色"));
        assert!(input.chars().count() <= SUMMARY_CHAR_LIMIT);

        let many: Vec<String> = (0..200)
            .map(|index| format!("问题 {index} {}", "字".repeat(80)))
            .collect();
        let clipped = build_summary_input(&many, &"尾".repeat(80));
        assert!(clipped.chars().count() <= SUMMARY_CHAR_LIMIT);
        assert!(clipped.contains("问题 199"));
        assert!(!clipped.contains("问题 0 "));
        assert!(clipped.contains('尾'));

        let huge = "问".repeat(10_000);
        let one = build_summary_input(&[huge], &"结尾标记".repeat(8));
        assert!(one.chars().count() <= SUMMARY_CHAR_LIMIT);
        assert!(one.contains("结尾标记"));
    }

    #[test]
    fn api_key_prefers_env_then_files_and_ignores_empty() {
        let dir = std::env::temp_dir().join(format!(
            "summary-key-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let appdata = dir.join("from-appdata");
        let beside = dir.join("from-exe");
        std::fs::write(&appdata, "\u{feff} appdata-key \nsecond\n").unwrap();
        std::fs::write(&beside, "exe-key\n").unwrap();
        let files = vec![appdata, beside.clone()];
        assert_eq!(
            load_api_key_from(Some("  env-key  "), &files).as_deref(),
            Some("env-key")
        );
        assert_eq!(
            load_api_key_from(Some("   "), &files).as_deref(),
            Some("appdata-key")
        );
        assert_eq!(
            load_api_key_from(None, &[beside]).as_deref(),
            Some("exe-key")
        );
        assert!(load_api_key_from(None, &[dir.join("missing")]).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
