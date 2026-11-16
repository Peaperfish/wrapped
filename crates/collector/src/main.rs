//! Wrapped collector binary. Phase A: wiring only; the platform event source,
//! tray and CLI land in Phase B.

mod platform;

fn main() {
    match platform::data_dir() {
        Some(dir) => println!(
            "wrapped-collector: database would be {}",
            dir.join("wrapped.db").display()
        ),
        None => println!("wrapped-collector: this platform is not supported yet"),
    }
    for c in wrapped_core::collectors::builtin() {
        let i = c.info();
        println!(
            "  collector {} (default {})",
            i.name,
            if i.default_on { "on" } else { "off" }
        );
    }
}
