//! Native modules that need more than the render core may link: the
//! keychain of the system and SQLite.

pub mod secure_storage;
#[cfg(feature = "sqlite")]
pub mod sqlite;

use std::path::Path;
use std::sync::Arc;

use pam_desktop_ui::modules::Module;
use pam_desktop_ui::modules::fetch::HttpClient;

/// The modules the host binary adds to the runtime, by the name PHP calls
/// them: `secure-storage` and, with the `sqlite` feature, `sqlite`.
///
/// `data` and `cache` are the application's directories and `application_id`
/// names it in the keychain. The cache directory, the HTTP client and the
/// development flag are not used by today's modules.
#[must_use]
pub fn host_modules(
    data: &Path,
    cache: &Path,
    application_id: &str,
    client: Arc<dyn HttpClient>,
    development: bool,
) -> Vec<(String, Box<dyn Module>)> {
    let _ = (cache, client, development);
    let secure_storage: Box<dyn Module> =
        Box::new(secure_storage::SecureStorage::new(data, application_id));
    #[cfg(feature = "sqlite")]
    let sqlite: Box<dyn Module> = Box::new(sqlite::Sqlite::new(data));
    vec![
        (String::from("secure-storage"), secure_storage),
        #[cfg(feature = "sqlite")]
        (String::from("sqlite"), sqlite),
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
        );
        let names: Vec<&str> = modules.iter().map(|(name, _)| name.as_str()).collect();
        assert!(names.contains(&"secure-storage"));
        assert_eq!(names.contains(&"sqlite"), cfg!(feature = "sqlite"));
        let _ = std::fs::remove_dir_all(&directory);
    }
}
