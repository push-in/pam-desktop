//! Presentation controllers: what the mobile hosts implement with native
//! navigation controllers, tab bars and dialogs is done here once, on top of
//! plain scene nodes, so every desktop backend behaves the same.

pub mod coverage;
pub mod modal;
pub mod navigation;
pub mod tabs;
