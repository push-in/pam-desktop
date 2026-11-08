//! Native modules that need more than the render core may link: the
//! keychain of the system, SQLite and signed updates.

pub mod secure_storage;
#[cfg(feature = "sqlite")]
pub mod sqlite;
pub mod updates;
pub mod websocket;

use std::path::Path;
use std::sync::Arc;

use pam_desktop_ui::modules::Module;
use pam_desktop_ui::modules::fetch::HttpClient;

use crate::bundle::Installed;
use crate::manifest::Manifest;

/// The modules the host binary adds to the runtime, by the name PHP calls
/// them: `secure-storage`, `updates` and, with the `sqlite` feature, `sqlite`.
///
/// `data` and `cache` are the application's directories and `application_id`
/// names it in the keychain and in the update feed. `manifest` carries the
/// version and the `updates` block. The cache directory is not used by
/// today's modules.
#[must_use]
pub fn host_modules(
    data: &Path,
    cache: &Path,
    application_id: &str,
    client: Arc<dyn HttpClient>,
    development: bool,
    manifest: &Manifest,
) -> Vec<(String, Box<dyn Module>)> {
    let _ = cache;
    let secure_storage: Box<dyn Module> =
        Box::new(secure_storage::SecureStorage::new(data, application_id));
    #[cfg(feature = "sqlite")]
    let sqlite: Box<dyn Module> = Box::new(sqlite::Sqlite::new(data));
    // A feed the manifest did not describe correctly is not a feed to trust.
    let update_config = manifest.updates().unwrap_or_else(|error| {
        eprintln!(
            "pam-desktop: {}: {error}; updates are disabled",
            crate::manifest::FILE_NAME
        );
        None
    });
    let updates: Box<dyn Module> = Box::new(updates::Updates::new(
        application_id,
        &manifest.text("version").unwrap_or_default(),
        update_config,
        Installed::discover(application_id),
        client,
        development,
        // The update helper replaces the bundle once this process is gone.
        updates::exit_for_update,
    ));
    vec![
        (
            String::from("websocket"),
            Box::new(websocket::WebSockets::new(development)),
        ),
        (String::from("secure-storage"), secure_storage),
        #[cfg(feature = "sqlite")]
        (String::from("sqlite"), sqlite),
        (String::from("updates"), updates),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use pam_desktop_ui::modules::test_support::temporary_directory;

    #[test]
    fn the_host_registers_its_modules_by_name() {
        let directory = temporary_directory("host-modules");
        let client: Arc<dyn HttpClient> = Arc::new(crate::http_client::UreqClient::new());
        let modules = host_modules(
            &directory.join("data"),
            &directory.join("cache"),
            "dev.pam.desktop.tests",
            client,
            true,
            &Manifest::load(&directory),
        );
        let names: Vec<&str> = modules.iter().map(|(name, _)| name.as_str()).collect();
        assert!(names.contains(&"secure-storage"));
        assert!(names.contains(&"updates"));
        assert_eq!(names.contains(&"sqlite"), cfg!(feature = "sqlite"));
        let _ = std::fs::remove_dir_all(&directory);
    }
}
