pub mod command;
pub mod null;

#[cfg(target_os = "macos")]
pub mod sketchybar;

pub use command::CommandSink;
pub use null::NullSink;

#[cfg(target_os = "macos")]
pub use sketchybar::SketchybarSink;
