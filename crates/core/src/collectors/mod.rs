//! Built-in collectors.
//!
//! Adding a collector: create `my_thing.rs` with a `Default` type implementing
//! [`Collector`](crate::collector::Collector), then add `my_thing::MyThing` to
//! the `register!` line below. Nothing else in the app needs to change.

macro_rules! register {
    ($($module:ident :: $ty:ident),* $(,)?) => {
        $(pub mod $module; pub use $module::$ty;)*

        /// One fresh instance of every built-in collector, in a stable order.
        pub fn builtin() -> Vec<Box<dyn crate::collector::Collector>> {
            vec![$(Box::new($module::$ty::default())),*]
        }
    };
}

register!(
    app_usage::AppUsage,
    active_idle::ActiveIdle,
    git::Git,
    photos::Photos,
    disk_memory::DiskMemory,
    ai_usage::AiUsage,
);

/// Like [`builtin`], but AI usage looks for tools in a home folder that does
/// not exist, so tests and benchmarks never read the real logs of whoever runs
/// them.
pub fn builtin_hermetic() -> Vec<Box<dyn crate::collector::Collector>> {
    let env = crate::ai::Env::from_map(
        std::path::PathBuf::from("/nonexistent-wrapped-test-home"),
        &[],
    );
    let mut all = builtin();
    for c in &mut all {
        if c.info().name == ai_usage::NAME {
            *c = Box::new(AiUsage::with_env(env.clone()));
        }
    }
    all
}
