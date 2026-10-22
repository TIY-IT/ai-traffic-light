#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Before Tauri, plugins, and COM. A windows-subsystem exe has no console,
    // so this file is the only record that the process entered `main`.
    ai_traffic_light_lib::init_logging();
    ai_traffic_light_lib::run()
}
