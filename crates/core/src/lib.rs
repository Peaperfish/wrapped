//! Wrapped core: everything that is not platform code.
//!
//! * [`ai`]: AI usage: adapters for the tools' logs, sources, the scan driver.
//! * [`event`]: the system event vocabulary the platform layer produces.
//! * [`collector`]: the plugin trait, the [`collector::Sink`] and buffer.
//! * [`presence`]: the shared idle / sleep / lock state machine.
//! * [`probe`]: system counters (disk, memory) read through the platform layer.
//! * [`collectors`]: the built-in collectors and their registration list.
//! * [`ctl`]: the request-file channel from the dashboard / CLI to the collector.
//! * [`scan`]: folder walk, hashing and bounded EXIF reading for scanned collectors.
//! * [`store`]: SQLite schema, migrations, batched writes and daily rollups.
//! * [`engine`]: routes events, owns the flush policy, pause and settings.
//! * [`stats`]: the stats engine: totals, series, top-N, comparisons and
//!   records for any range of local dates (dashboard, recaps).

pub mod ai;
pub mod collector;
pub mod collectors;
pub mod ctl;
pub mod engine;
pub mod event;
pub mod presence;
pub mod probe;
pub mod scan;
pub mod settings;
pub mod stats;
pub mod store;
pub mod time;

pub use collector::{Collector, CollectorInfo, Kind, MetricDef, Sink};
pub use engine::Engine;
pub use event::{DisplayAssertion, SystemEvent};
pub use settings::{Pause, Settings};
pub use store::Store;
pub use time::{Clock, ManualClock, SystemClock, Timestamp};
