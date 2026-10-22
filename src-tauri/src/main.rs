// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Some(code) = voice_lib::self_test(&args) {
        return code;
    }
    voice_lib::run();
    std::process::ExitCode::SUCCESS
}
