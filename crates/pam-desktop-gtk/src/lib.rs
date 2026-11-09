//! GTK 4 backend of PAM Desktop.
//!
//! Every scene node is backed by a real widget: boxes, text and images are
//! `PamView` widgets (a `GtkWidget` subclass that paints with GSK render
//! nodes and Pango), scrolling uses `GtkScrolledWindow`, and text entry uses
//! `GtkText`/`GtkTextView`, so input methods, accessibility and theming come
//! from the toolkit.

#![cfg(target_os = "linux")]

mod accessibility;
mod backend;
mod compat;
mod fonts;
mod glyphs;
#[cfg(feature = "hotkeys")]
mod hotkeys;
mod input;
mod media;
mod paint;
mod printing;
mod raster;
mod share;
mod shell;
mod system;
mod text;
#[cfg(feature = "tray")]
mod tray;
mod trim;
mod view;
mod webview;
mod x11;

pub use backend::{GtkBackend, run};
