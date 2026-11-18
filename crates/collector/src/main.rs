//! Wrapped collector: tray app + temporary CLI.

mod cli;
mod platform;

/// Rust ignores SIGPIPE, so `println!` into a closed pipe (`... | head -1`)
/// panics with "Broken pipe" (Windows: "os error 232" / "os error 109"). That
/// is not an error for a CLI: exit quietly. Only `print!` family panics count;
/// other failures (disk full, ...) and other panics are reported as before.
fn exit_quietly_on_broken_pipe() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let msg = info
            .payload()
            .downcast_ref::<String>()
            .map(String::as_str)
            .or_else(|| info.payload().downcast_ref::<&str>().copied())
            .unwrap_or("");
        if is_closed_pipe_panic(msg) {
            std::process::exit(0);
        }
        default(info);
    }));
}

fn is_closed_pipe_panic(msg: &str) -> bool {
    msg.starts_with("failed printing to std")
        && ["Broken pipe", "os error 232", "os error 109"]
            .iter()
            .any(|m| msg.contains(m))
}

fn main() {
    exit_quietly_on_broken_pipe();
    std::process::exit(cli::main());
}

#[cfg(test)]
mod tests {
    use super::is_closed_pipe_panic;

    #[test]
    fn only_closed_pipe_print_panics_match() {
        assert!(is_closed_pipe_panic(
            "failed printing to stdout: Broken pipe (os error 32)"
        ));
        assert!(is_closed_pipe_panic(
            "failed printing to stdout: The pipe is being closed. (os error 232)"
        ));
        assert!(is_closed_pipe_panic(
            "failed printing to stderr: The pipe has been ended. (os error 109)"
        ));
        assert!(!is_closed_pipe_panic(
            "failed printing to stdout: No space left on device (os error 28)"
        ));
        assert!(!is_closed_pipe_panic(
            "called `Result::unwrap()` on an `Err` value: Broken pipe"
        ));
    }
}
