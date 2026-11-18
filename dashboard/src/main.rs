// No console window behind the dashboard in Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    std::process::exit(wrapped_dashboard::main());
}
