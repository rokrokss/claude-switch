pub mod cli;
pub mod clone;
pub mod clone_watch;
pub mod gui;
pub mod icons;
pub mod link_handler;
pub mod plist;
pub mod script;
pub mod shim;
#[cfg(target_os = "macos")]
mod system_icon;
pub mod wrapper;
