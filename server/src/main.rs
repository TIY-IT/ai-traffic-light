//! AI Traffic Light 团队面板:接收成员机器的状态灯客户端上报,供管理员在浏览器查看。
//! 内网明文 HTTP 是有意取舍;数据只含软件名/状态/项目名,不含会话内容。
use rusqlite::Connection;
use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const ONLINE_WINDOW_SECS: i64 = 90;

#[derive(Debug, Deserialize)]
struct ReportPayload {
    #[serde(rename = "deviceToken")]
    device_token: String,
    #[serde(rename = "displayName")]
    display_name: String,
    host: String,
    #[serde(rename = "activeClients", default)]
    active_clients: Vec<String>,
    #[serde(default)]
    sessions: Vec<ReportSession>,
    #[serde(rename = "reportedAt", default)]
    reported_at: i64,
}

#[derive(Debug, Deserialize, serde::Serialize)]
struct ReportSession {
    client: String,
    #[serde(default)]
    title: String,
    state: String,
    #[serde(rename = "updatedAt", default)]
    updated_at: i64,
}

struct App {
    db: Connection,
    admin_token: String,
}

struct Resp {
    status: u16,
    content_type: &'static str,
    body: String,
}

impl Resp {
    fn json(status: u16, body: String) -> Self {
        Self { status, content_type: "application/json; charset=utf-8", body }
    }
    fn empty(status: u16) -> Self {
        Self { status, content_type: "text/plain; charset=utf-8", body: String::new() }
    }
    fn text(status: u16, content_type: &'static str, body: String) -> Self {
        Self { status, content_type, body }
    }
}

fn now_secs() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn clean_text(value: &str, max: usize) -> String {
    value.trim().chars().take(max).collect()
}

impl App {
    fn open(db_path: &Path, admin_token: String) -> rusqlite::Result<Self> {
        if let Some(parent) = db_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let db = Connection::open(db_path)?;
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS devices(
                device_token TEXT PRIMARY KEY,
                display_name TEXT NOT NULL,
                host TEXT NOT NULL,
                active_clients TEXT NOT NULL,
                sessions TEXT NOT NULL,
                last_seen INTEGER NOT NULL,
                created_at INTEGER NOT NULL
            );",
        )?;
        Ok(Self { db, admin_token })
    }

    fn report(&self, payload: &ReportPayload) -> rusqlite::Result<()> {
        let token = clean_text(&payload.device_token, 128);
        if token.is_empty() {
            return Ok(());
        }
        let host = clean_text(&payload.host, 128);
        let display_name = {
            let name = clean_text(&payload.display_name, 64);
            if name.is_empty() { if host.is_empty() { "未命名设备".into() } else { host.clone() } } else { name }
        };
        let clients = serde_json::to_string(&payload.active_clients).unwrap_or_else(|_| "[]".into());
        let sessions = serde_json::to_string(&payload.sessions).unwrap_or_else(|_| "[]".into());
        let now = if payload.reported_at > 0 { payload.reported_at } else { now_secs() };
        self.db.execute(
            "INSERT INTO devices(device_token, display_name, host, active_clients, sessions, last_seen, created_at)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?6)
             ON CONFLICT(device_token) DO UPDATE SET
                display_name = excluded.display_name,
                host = excluded.host,
                active_clients = excluded.active_clients,
                sessions = excluded.sessions,
                last_seen = excluded.last_seen",
            rusqlite::params![token, display_name, host, clients, sessions, now],
        )?;
        Ok(())
    }

    fn overview(&self) -> rusqlite::Result<serde_json::Value> {
        let mut stmt = self.db.prepare(
            "SELECT device_token, display_name, host, active_clients, sessions, last_seen, created_at
             FROM devices ORDER BY display_name",
        )?;
        let now = now_secs();
        let rows = stmt.query_map([], |row| {
            let last_seen: i64 = row.get(5)?;
            let clients_raw: String = row.get(3)?;
            let sessions_raw: String = row.get(4)?;
            Ok(serde_json::json!({
                "deviceToken": row.get::<_, String>(0)?,
                "displayName": row.get::<_, String>(1)?,
                "host": row.get::<_, String>(2)?,
                "activeClients": serde_json::from_str::<serde_json::Value>(&clients_raw).unwrap_or_else(|_| serde_json::json!([])),
                "sessions": serde_json::from_str::<serde_json::Value>(&sessions_raw).unwrap_or_else(|_| serde_json::json!([])),
                "lastSeen": last_seen,
                "createdAt": row.get::<_, i64>(6)?,
                "online": now - last_seen <= ONLINE_WINDOW_SECS,
            }))
        })?;
        let devices: Vec<serde_json::Value> = rows.collect::<Result<_, _>>()?;
        Ok(serde_json::json!({ "serverTime": now, "devices": devices }))
    }

    fn rename_device(&self, token: &str, display_name: &str) -> rusqlite::Result<bool> {
        let name = clean_text(display_name, 64);
        if name.is_empty() || token.is_empty() {
            return Ok(false);
        }
        let changed = self.db.execute(
            "UPDATE devices SET display_name = ?2 WHERE device_token = ?1",
            rusqlite::params![token, name],
        )?;
        Ok(changed > 0)
    }

    fn delete_device(&self, token: &str) -> rusqlite::Result<bool> {
        let changed = self.db.execute(
            "DELETE FROM devices WHERE device_token = ?1",
            rusqlite::params![token],
        )?;
        Ok(changed > 0)
    }

    fn authorize(&self, auth_header: Option<&str>) -> bool {
        match auth_header {
            Some(value) => value.trim() == format!("Bearer {}", self.admin_token),
            None => false,
        }
    }
}

/// 纯函数式路由处理:输入方法/路径/鉴权头/请求体,输出响应。HTTP 层只做搬运。
fn handle(app: &App, method: &str, path: &str, auth: Option<&str>, body: &str) -> Resp {
    let path = path.split('?').next().unwrap_or(path);
    match (method, path) {
        ("GET", "/") | ("GET", "/index.html") => Resp::text(
            200,
            "text/html; charset=utf-8",
            include_str!("web/index.html").to_string(),
        ),
        ("POST", "/api/v1/report") => match serde_json::from_str::<ReportPayload>(body) {
            Ok(payload) => match app.report(&payload) {
                Ok(()) => Resp::empty(204),
                Err(_) => Resp::json(500, r#"{"error":"storage"}"#.into()),
            },
            Err(_) => Resp::json(400, r#"{"error":"bad payload"}"#.into()),
        },
        ("GET", "/api/v1/overview") => {
            if !app.authorize(auth) {
                return Resp::json(401, r#"{"error":"unauthorized"}"#.into());
            }
            match app.overview() {
                Ok(value) => Resp::json(200, value.to_string()),
                Err(_) => Resp::json(500, r#"{"error":"storage"}"#.into()),
            }
        }
        (method, path) if path.starts_with("/api/v1/devices/") => {
            if !app.authorize(auth) {
                return Resp::json(401, r#"{"error":"unauthorized"}"#.into());
            }
            let token = path.trim_start_matches("/api/v1/devices/");
            match method {
                "PATCH" => match serde_json::from_str::<serde_json::Value>(body) {
                    Ok(value) => {
                        let name = value["displayName"].as_str().unwrap_or_default();
                        match app.rename_device(token, name) {
                            Ok(true) => Resp::empty(204),
                            Ok(false) => Resp::json(404, r#"{"error":"not found"}"#.into()),
                            Err(_) => Resp::json(500, r#"{"error":"storage"}"#.into()),
                        }
                    }
                    Err(_) => Resp::json(400, r#"{"error":"bad payload"}"#.into()),
                },
                "DELETE" => match app.delete_device(token) {
                    Ok(true) => Resp::empty(204),
                    Ok(false) => Resp::json(404, r#"{"error":"not found"}"#.into()),
                    Err(_) => Resp::json(500, r#"{"error":"storage"}"#.into()),
                },
                _ => Resp::json(404, r#"{"error":"not found"}"#.into()),
            }
        }
        _ => Resp::json(404, r#"{"error":"not found"}"#.into()),
    }
}

fn generate_token() -> String {
    use rand::Rng;
    let mut rng = rand::rng();
    (0..32).map(|_| format!("{:x}", rng.random_range(0u8..16))).collect()
}

fn data_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn load_or_create_admin_token(dir: &Path) -> std::io::Result<String> {
    let file = dir.join("admin-token.txt");
    if let Ok(existing) = std::fs::read_to_string(&file) {
        let token = existing.trim().to_string();
        if !token.is_empty() {
            return Ok(token);
        }
    }
    let token = generate_token();
    std::fs::write(&file, &token)?;
    Ok(token)
}

/// 尽力而为地取局域网地址用于控制台提示;失败就回退 localhost。
fn lan_hint(port: u16) -> Option<String> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("10.255.255.255:1").ok()?;
    socket.local_addr().ok().map(|a| format!("http://{}:{port}", a.ip()))
}

fn main() {
    let mut port: u16 = 8787;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--port" {
            if let Ok(value) = args.next().unwrap_or_default().parse() {
                port = value;
            }
        }
    }

    let dir = data_dir();
    let admin_token = match load_or_create_admin_token(&dir) {
        Ok(token) => token,
        Err(err) => {
            eprintln!("无法写入 admin-token.txt({err});请检查目录权限或换目录运行。");
            std::process::exit(1);
        }
    };
    let app = match App::open(&dir.join("server.db"), admin_token.clone()) {
        Ok(app) => app,
        Err(err) => {
            eprintln!("无法打开 server.db({err});请检查目录权限或换目录运行。");
            std::process::exit(1);
        }
    };

    let server = match tiny_http::Server::http(format!("0.0.0.0:{port}")) {
        Ok(server) => server,
        Err(err) => {
            eprintln!("端口 {port} 监听失败({err});可能被占用,可用 --port 换端口。");
            std::process::exit(1);
        }
    };

    println!("AI 状态灯 · 团队面板已启动");
    println!("管理页:   {}", lan_hint(port).unwrap_or_else(|| format!("http://localhost:{port}")));
    println!("管理密钥: {admin_token}(已保存到 admin-token.txt,请勿外发)");

    for mut request in server.incoming_requests() {
        let method = request.method().as_str().to_string();
        let url = request.url().to_string();
        let auth = request
            .headers()
            .iter()
            .find(|h| h.field.equiv("Authorization"))
            .and_then(|h| h.value.as_str().to_string().into());
        let body = if method == "POST" || method == "PATCH" {
            use std::io::Read;
            let mut buf = String::new();
            let _ = request.as_reader().read_to_string(&mut buf);
            buf
        } else {
            String::new()
        };
        let resp = handle(&app, &method, &url, auth.as_deref(), &body);
        let header =
            tiny_http::Header::from_bytes("Content-Type", resp.content_type).unwrap();
        let _ = request.respond(
            tiny_http::Response::from_data(resp.body.as_bytes())
                .with_status_code(resp.status)
                .with_header(header),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_app() -> App {
        static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir()
            .join(format!("ai-light-server-test-{}-{seq}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        App::open(&dir.join("server.db"), "admin-secret".into()).unwrap()
    }

    fn payload(token: &str, name: &str, reported_at: i64) -> String {
        serde_json::json!({
            "deviceToken": token,
            "displayName": name,
            "host": "PC-01",
            "activeClients": ["codex", "zcode"],
            "sessions": [
                {"client": "codex", "title": "demo · abc123", "state": "working", "updatedAt": 1700000000i64}
            ],
            "reportedAt": reported_at
        })
        .to_string()
    }

    #[test]
    fn report_then_overview_shows_online_device() {
        let app = test_app();
        let resp = handle(&app, "POST", "/api/v1/report", None, &payload("dev-1", "张三", now_secs()));
        assert_eq!(resp.status, 204);

        let resp = handle(&app, "GET", "/api/v1/overview", Some("Bearer admin-secret"), "");
        assert_eq!(resp.status, 200);
        let value: serde_json::Value = serde_json::from_str(&resp.body).unwrap();
        let devices = value["devices"].as_array().unwrap();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0]["displayName"], "张三");
        assert_eq!(devices[0]["activeClients"].as_array().unwrap().len(), 2);
        assert_eq!(devices[0]["online"], true);
        assert_eq!(devices[0]["sessions"][0]["state"], "working");
    }

    #[test]
    fn overview_requires_admin_token() {
        let app = test_app();
        assert_eq!(handle(&app, "GET", "/api/v1/overview", None, "").status, 401);
        assert_eq!(handle(&app, "GET", "/api/v1/overview", Some("Bearer wrong"), "").status, 401);
    }

    #[test]
    fn report_upsert_keeps_created_at_and_renames() {
        let app = test_app();
        handle(&app, "POST", "/api/v1/report", None, &payload("dev-1", "张三", 1700000000));
        let mut second: serde_json::Value = serde_json::from_str(&payload("dev-1", "张三", 1700000000)).unwrap();
        second["displayName"] = "李四".into();
        second["reportedAt"] = 1700000050i64.into();
        handle(&app, "POST", "/api/v1/report", None, &second.to_string());

        let value: serde_json::Value =
            serde_json::from_str(&handle(&app, "GET", "/api/v1/overview", Some("Bearer admin-secret"), "").body).unwrap();
        let devices = value["devices"].as_array().unwrap();
        assert_eq!(devices.len(), 1, "同 token 应去重");
        assert_eq!(devices[0]["displayName"], "李四");
        assert_eq!(devices[0]["createdAt"], 1700000000i64, "created_at 不应被覆盖");
        assert_eq!(devices[0]["lastSeen"], 1700000050i64);
    }

    #[test]
    fn rename_and_delete_devices() {
        let app = test_app();
        handle(&app, "POST", "/api/v1/report", None, &payload("dev-1", "张三", now_secs()));

        let resp = handle(&app, "PATCH", "/api/v1/devices/dev-1", Some("Bearer admin-secret"), r#"{"displayName":"王五"}"#);
        assert_eq!(resp.status, 204);
        assert_eq!(handle(&app, "PATCH", "/api/v1/devices/dev-x", Some("Bearer admin-secret"), r#"{"displayName":" nobody"}"#).status, 404);
        assert_eq!(handle(&app, "PATCH", "/api/v1/devices/dev-1", Some("Bearer admin-secret"), r#"{"displayName":""}"#).status, 404);

        let value: serde_json::Value =
            serde_json::from_str(&handle(&app, "GET", "/api/v1/overview", Some("Bearer admin-secret"), "").body).unwrap();
        assert_eq!(value["devices"][0]["displayName"], "王五");

        assert_eq!(handle(&app, "DELETE", "/api/v1/devices/dev-1", Some("Bearer admin-secret"), "").status, 204);
        assert_eq!(handle(&app, "DELETE", "/api/v1/devices/dev-1", Some("Bearer admin-secret"), "").status, 404);
        let value: serde_json::Value =
            serde_json::from_str(&handle(&app, "GET", "/api/v1/overview", Some("Bearer admin-secret"), "").body).unwrap();
        assert_eq!(value["devices"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn bad_payloads_are_rejected_and_index_is_public() {
        let app = test_app();
        assert_eq!(handle(&app, "POST", "/api/v1/report", None, "not json").status, 400);
        assert_eq!(handle(&app, "POST", "/api/v1/report", None, r#"{"host":"x"}"#).status, 400);
        let index = handle(&app, "GET", "/", None, "");
        assert_eq!(index.status, 200);
        assert!(index.body.contains("团队面板"));
        assert_eq!(handle(&app, "GET", "/api/unknown", None, "").status, 404);
    }

    #[test]
    fn offline_devices_reported_as_offline() {
        let app = test_app();
        // 上报时间拨到 1 小时前,超过 90 秒在线窗口
        handle(&app, "POST", "/api/v1/report", None, &payload("dev-old", "老王", now_secs() - 3600));
        let value: serde_json::Value =
            serde_json::from_str(&handle(&app, "GET", "/api/v1/overview", Some("Bearer admin-secret"), "").body).unwrap();
        assert_eq!(value["devices"][0]["online"], false);
    }
}
