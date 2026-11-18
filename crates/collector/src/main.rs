//! Wrapped collector: tray app + temporary CLI.

mod cli;
mod platform;

/// Rust ignores SIGPIPE, so `println!` into a closed pipe (`... | head -1`)
/// panics with "Broken pipe". That is not an error for a CLI: exit quietly.
fn exit_quietly_on_broken_pipe() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let msg = info
            .payload()
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| info.payload().downcast_ref::<&str>().copied())
            .unwrap_or("");
        if msg.contains("Broken pipe") {
            std::process::exit(0);
        }
        default(info);
    }));
}

fn main() {
    exit_quietly_on_broken_pipe();
    std::process::exit(cli::main());
}
