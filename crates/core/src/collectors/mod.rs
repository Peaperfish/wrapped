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
);
