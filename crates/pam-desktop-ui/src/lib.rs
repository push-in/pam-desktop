//! Platform-neutral render core of PAM Desktop.
//!
//! The PHP worker produces tree and patch frames, the layout engine turns
//! them into mutations, and this crate keeps the retained scene and drives a
//! [`backend::Backend`] (GTK, AppKit or Win32) with resolved, ready-to-paint
//! state.

pub mod animator;
pub mod backend;
pub mod color;
pub mod context;
pub mod geometry;
pub mod interaction;
pub mod presenter;
pub mod props;
pub mod runtime;
pub mod scene;
pub mod style;
pub mod testing;
pub mod text;
pub mod watch;
pub mod wiremap;
pub mod worker;

pub use pam_desktop_engine as engine;
pub use pam_desktop_wire as wire;
