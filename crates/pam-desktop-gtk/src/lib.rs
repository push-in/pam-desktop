//! GTK 4 backend of PAM Desktop.
//!
//! Every scene node is backed by a real widget: boxes, text and images are
//! `PamView` widgets (a `GtkWidget` subclass that paints with GSK render
//! nodes and Pango), scrolling uses `GtkScrolledWindow`, and text entry uses
//! `GtkText`/`GtkTextView`, so input methods, accessibility and theming come
//! from the toolkit.

#![cfg(target_os = "linux")]

mod backend;
mod input;
mod paint;
mod text;
mod view;

pub use backend::{run, GtkBackend};
