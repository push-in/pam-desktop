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
mod input;
mod media;
mod paint;
mod shell;
mod text;
#[cfg(feature = "tray")]
mod tray;
mod view;
#[cfg(feature = "webview")]
mod webview;

pub use backend::{run, GtkBackend};
