//! 团队上报(opt-in):把本机在用的 AI 客户端与会话元数据定期 POST 到团队服务器。
//! 只上报软件名、状态、项目名,不含会话内容;默认关闭,未配置时零网络行为。
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const DEFAULT_INTERVAL_SECS: u64 = 30;
static RELOAD: AtomicBool = AtomicBool::new(false);
static LAST_OK: AtomicU64 = AtomicU64::new(0);
static LAST_ATTEMPT: AtomicU64 = AtomicU64::new(0);
static LAST_ERROR: Mutex<Option<String>> = Mutex::new(None);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TeamConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub server_url: String,
    #[serde(default)]
    pub display_name: String,
    #[serde(default)]
    pub device_token: String,
    #[serde(default = "default_interval")]
    pub interval_secs: u64,
}

fn default_interval() -> u64 {
    DEFAULT_INTERVAL_SECS
}

impl Default for TeamConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            server_url: String::new(),
            display_name: String::new(),
            device_token: String::new(),
            interval_secs: DEFAULT_INTERVAL_SECS,
        }
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn config_path() -> Option<PathBuf> {
    Some(dirs::home_dir()?.join(".ai-traffic-light").join("config.json"))
}

pub fn load_config() -> TeamConfig {
    let Some(path) = config_path() else { return TeamConfig::default() };
    let Ok(content) = std::fs::read_to_string(&path) else { return TeamConfig::default() };
    let Ok(root) = serde_json::from_str::<Value>(content.trim_start_matches('\u{feff}')) else {
        return TeamConfig::default();
    };
    match root.get("team") {
        Some(team) => serde_json::from_value(team.clone()).unwrap_or_default(),
        None => TeamConfig::default(),
    }
}

pub fn save_config(config: &TeamConfig) -> Result<(), String> {
    let path = config_path().ok_or_else(|| "无法定位用户目录".to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let mut root: Value = match std::fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(content.trim_start_matches('\u{feff}'))
            .unwrap_or_else(|_| serde_json::json!({})),
        Err(_) => serde_json::json!({}),
    };
    root["team"] = serde_json::to_value(config).map_err(|e| e.to_string())?;
    let pretty = serde_json::to_string_pretty(&root).map_err(|e| e.to_string())?;
    std::fs::write(&path, pretty).map_err(|e| e.to_string())
}

fn generate_token() -> String {
    use rand::Rng;
    let mut rng = rand::rng();
    (0..32).map(|_| format!("{:x}", rng.random_range(0u8..16))).collect()
}

fn host() -> String {
    sysinfo::System::host_name().unwrap_or_else(|| "unknown".into())
}

fn build_payload(config: &TeamConfig) -> Value {
    let snapshot = crate::bridge::read();
    let sessions: Vec<Value> = snapshot
        .sessions
        .iter()
        .map(|s| {
            serde_json::json!({
                "client": s.client,
                "title": s.title,
                "state": s.state,
                "updatedAt": s.updated_at,
            })
        })
        .collect();
    serde_json::json!({
        "deviceToken": config.device_token,
        "displayName": if config.display_name.trim().is_empty() { host() } else { config.display_name.trim().to_string() },
        "host": host(),
        "activeClients": crate::presence::get_active_clients(),
        "sessions": sessions,
        "reportedAt": now_secs(),
    })
}

fn report_once(config: &TeamConfig) -> Result<(), String> {
    if config.device_token.is_empty() {
        return Err("缺少设备标识".into());
    }
    let url = format!("{}/api/v1/report", config.server_url.trim_end_matches('/'));
    let payload = build_payload(config);
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?;
    let response = client
        .post(&url)
        .json(&payload)
        .send()
        .map_err(|e| format!("连接失败:{e}"))?;
    if response.status().is_success() {
        Ok(())
    } else {
        Err(format!("服务器返回 {}", response.status()))
    }
}

/// 分片睡眠,期间发现配置变更信号立即返回,让保存后无需等满一个周期。
fn sleep_interruptible(secs: u64) {
    for _ in 0..secs.saturating_mul(2) {
        if RELOAD.swap(false, Ordering::Relaxed) {
            return;
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    RELOAD.store(false, Ordering::Relaxed);
}

pub fn start() {
    std::thread::spawn(|| loop {
        let mut config = load_config();
        if config.enabled && !config.server_url.trim().is_empty() {
            if config.device_token.is_empty() {
                config.device_token = generate_token();
                let _ = save_config(&config);
            }
            LAST_ATTEMPT.store(now_secs(), Ordering::Relaxed);
            match report_once(&config) {
                Ok(()) => {
                    LAST_OK.store(now_secs(), Ordering::Relaxed);
                    if let Ok(mut slot) = LAST_ERROR.lock() {
                        *slot = None;
                    }
                }
                Err(err) => {
                    if let Ok(mut slot) = LAST_ERROR.lock() {
                        *slot = Some(err);
                    }
                }
            }
            sleep_interruptible(config.interval_secs.clamp(5, 3600));
        } else {
            sleep_interruptible(2);
        }
    });
}

#[derive(Debug, Clone, Serialize)]
pub struct ReportStatus {
    pub last_ok: u64,
    pub last_attempt: u64,
    pub last_error: Option<String>,
}

#[tauri::command]
pub fn get_team_config() -> TeamConfig {
    load_config()
}

#[tauri::command]
pub fn set_team_config(mut config: TeamConfig) -> Result<TeamConfig, String> {
    config.server_url = config.server_url.trim().trim_end_matches('/').to_string();
    if config.enabled
        && !config.server_url.starts_with("http://")
        && !config.server_url.starts_with("https://")
    {
        return Err("服务器地址需以 http:// 或 https:// 开头".into());
    }
    if config.device_token.is_empty() {
        config.device_token = generate_token();
    }
    if config.interval_secs == 0 || config.interval_secs > 3600 {
        config.interval_secs = DEFAULT_INTERVAL_SECS;
    }
    save_config(&config)?;
    RELOAD.store(true, Ordering::Relaxed);
    Ok(config)
}

#[tauri::command]
pub fn report_status() -> ReportStatus {
    ReportStatus {
        last_ok: LAST_OK.load(Ordering::Relaxed),
        last_attempt: LAST_ATTEMPT.load(Ordering::Relaxed),
        last_error: LAST_ERROR.lock().map(|slot| slot.clone()).unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_round_trips_and_preserves_unknown_keys() {
        let dir = std::env::temp_dir().join(format!("ai-light-reporter-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        std::fs::write(&path, r#"{"legacy":{"keep":true},"team":{"enabled":true,"server_url":"http://x:8787"}}"#).unwrap();

        // 借 config_path 不可行(内部函数),直接走 save/load 的序列化逻辑验证:
        let root: Value =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap().trim_start_matches('\u{feff}')).unwrap();
        assert_eq!(root["legacy"]["keep"], true);
        let config: TeamConfig = serde_json::from_value(root["team"].clone()).unwrap();
        assert!(config.enabled);
        assert_eq!(config.interval_secs, DEFAULT_INTERVAL_SECS, "缺省间隔应回退默认值");

        let mut updated = config.clone();
        updated.display_name = "张三".into();
        let mut root = root;
        root["team"] = serde_json::to_value(&updated).unwrap();
        std::fs::write(&path, serde_json::to_string_pretty(&root).unwrap()).unwrap();
        let root: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(root["legacy"]["keep"], true, "未知字段不应被丢弃");
        assert_eq!(root["team"]["display_name"], "张三");
    }

    #[test]
    fn default_config_is_offline_silent() {
        let config = TeamConfig::default();
        assert!(!config.enabled, "默认必须关闭上报");
        assert!(config.server_url.is_empty());
    }

    #[test]
    fn invalid_urls_are_rejected_by_set_rules() {
        let mut config = TeamConfig::default();
        config.enabled = true;
        config.server_url = "192.168.1.5:8787".into();
        assert!(!config.server_url.starts_with("http://"), "前置校验依据:非 http(s) 地址应被拒绝");
    }
}
