//! Windowless Windows entry point. The CLI keeps its console subsystem.
#![cfg_attr(windows, windows_subsystem = "windows")]
fn main() {
    #[cfg(windows)]
    if let Err(e) = std::env::current_exe().and_then(|exe| {
        let mut command =
            wrapped_core::background_command(exe.with_file_name("wrapped-collector.exe"));
        let args = std::env::args_os().skip(1).collect::<Vec<_>>();
        match args.as_slice() {
            [] => {
                command.arg("launch");
            }
            [flag, db] if flag == "--db" && !db.is_empty() => {
                command.arg("launch").env("WRAPPED_DB", db);
            }

            [activate, uri] if activate == "activate" => {
                command.arg(activate).arg(uri);
            }
            _ => return Err(std::io::Error::other("invalid launcher arguments")),
        }
        command.spawn()
    }) {
        eprintln!("Cannot launch Wrapped: {e}");
        std::process::exit(1);
    }
}
