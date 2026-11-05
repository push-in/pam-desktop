//! Embeds the Windows application manifest: the version 6 common controls
//! (themed inputs, task dialogs), per-monitor DPI awareness, long paths and
//! UTF-8 as the process code page. Other systems need nothing here.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        use embed_manifest::manifest::{ActiveCodePage, DpiAwareness, Setting, SupportedOS};
        use embed_manifest::{embed_manifest, new_manifest};

        let manifest = new_manifest("Pushin.PamDesktop")
            .supported_os(SupportedOS::Windows10..=SupportedOS::Windows10)
            .active_code_page(ActiveCodePage::Utf8)
            .dpi_awareness(DpiAwareness::PerMonitorV2)
            .long_path_aware(Setting::Enabled);
        if let Err(error) = embed_manifest(manifest) {
            println!("cargo:warning=the Windows manifest was not embedded: {error}");
        }
    }
}
