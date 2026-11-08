//! Win32 backend of PAM Desktop (Windows 10 1809+ and 11).
//!
//! One top-level window per `WindowId`, rendered as a whole with Direct2D
//! and DirectWrite on a flip-model swap chain. Views are a retained tree of
//! plain structs; only text inputs and web views are child windows.
#![cfg(target_os = "windows")]

mod accessibility;
mod app;
mod backend;
mod dragdrop;
mod events;
mod focus;
mod fonts;
mod gfx;
mod images;
mod inline_boxes;
mod input;
mod instance;
mod media;
mod menu_theme;
mod native;
mod paint;
mod printing;
mod render;
mod scene;
mod scroll;
mod share;
mod shell;
mod svg;
mod text;
mod util;
mod webview;
mod window;

pub use backend::{Win32Backend, run};
