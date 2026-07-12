pub mod command;
pub mod dbus;
pub mod null;

#[cfg(target_os = "macos")]
pub mod sketchybar;

pub use command::CommandSink;
pub use dbus::{DbusSink, DbusSinkConfig};
pub use null::NullSink;

#[cfg(target_os = "macos")]
pub use sketchybar::SketchybarSink;
