use crate::sessions::SessionSnapshot;
use std::{sync::Mutex, time::{Instant, Duration}, process::Command};
static STATUS: Mutex<Option<(Instant, Vec<SessionSnapshot>)>> = Mutex::new(None);

pub fn start() {
    std::thread::spawn(|| { loop {
        std::thread::sleep(Duration::from_secs(2));
        // Reuse Marvis's installed Python runtime; install nothing in the AI client.
        let mut command = Command::new("powershell.exe");
        command.args(["-NoProfile", "-NonInteractive", "-Command", "(Get-Process MarvisHost -ErrorAction SilentlyContinue | Select-Object -First 1 -ExpandProperty Path)"]);
        #[cfg(windows)] { use std::os::windows::process::CommandExt; command.creation_flags(0x08000000); }
        let host = match command.output() { Ok(o) => String::from_utf8_lossy(&o.stdout).trim().to_string(), Err(_) => continue };
        let Some(parent) = std::path::Path::new(&host).parent() else { continue };
        let python = parent.join("binaries/python/current/python.exe");
        if !python.exists() { continue; }
        loop {
            if !crate::presence::get_active_clients().iter().any(|c| c == "marvis") { break; }
            let mut command = Command::new(&python);
            command.args(["-I", "-c", include_str!("../resources/marvis-status.py")]);
            #[cfg(windows)] { use std::os::windows::process::CommandExt; command.creation_flags(0x08000000); }
            if let Ok(output) = command.output() {
                if output.status.success() {
                    if let Ok(sessions) = serde_json::from_slice::<Vec<SessionSnapshot>>(&output.stdout) {
                        if let Ok(mut status) = STATUS.lock() { *status = Some((Instant::now(), sessions)); }
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    }});
}

pub fn sessions() -> Vec<SessionSnapshot> {
    if let Ok(status) = STATUS.lock() {
        if let Some((at, sessions)) = status.as_ref() {
            if at.elapsed().as_secs() < 5 { return sessions.clone(); }
        }
    }
    Vec::new()
}
