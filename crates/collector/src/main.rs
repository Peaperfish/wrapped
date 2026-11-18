//! Wrapped collector: tray app + temporary CLI.

mod cli;
mod platform;

fn main() {
    std::process::exit(cli::main());
}
