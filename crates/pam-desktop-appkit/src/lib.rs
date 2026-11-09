//! AppKit backend of PAM Desktop.
//!
//! Every scene node is backed by a layer-backed `NSView`: boxes, text and
//! images are `PamView`s (painted with Core Graphics and TextKit, or with
//! plain layer properties when a box is simple enough), scrolling uses
//! `NSScrollView`, text entry uses `NSTextField`/`NSTextView`, media uses
//! AVFoundation and web pages use `WKWebView`, so input methods,
//! accessibility and system behaviour come from the platform.

#![cfg(target_os = "macos")]

mod accessibility;
mod backdrop;
mod backend;
mod capture;
mod clock;
mod events;
mod fonts;
mod image;
mod input;
mod media;
mod paint;
mod printing;
mod scroll;
mod shell;
mod text;
mod util;
mod view;
mod webview;
mod window;

pub use backend::{AppKitBackend, run};
