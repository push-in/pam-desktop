use std::path::{Component, Path, PathBuf};

use pam_desktop_protocol::Bootstrap;

#[derive(Clone, Debug)]
pub struct Project {
    root: PathBuf,
    application: PathBuf,
}

impl Project {
    pub fn discover(path: &Path) -> Result<Self, String> {
        let root = path
            .canonicalize()
            .map_err(|error| format!("cannot resolve project {}: {error}", path.display()))?;
        if !root.is_dir() {
            return Err(format!("project {} is not a directory", root.display()));
        }

        let application = root.join("app.php");
        if !application.is_file() {
            return Err(format!(
                "{} is missing; create the project with `pam init --template desktop`",
                application.display()
            ));
        }
        if !root.join("composer.json").is_file() {
            return Err(format!("{} has no composer.json", root.display()));
        }
        if !root.join("vendor/autoload.php").is_file() {
            return Err(
                "vendor/autoload.php is missing; run `composer install` in the project".to_owned(),
            );
        }

        Ok(Self { root, application })
    }

    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    #[must_use]
    pub fn application(&self) -> &Path {
        &self.application
    }

    pub fn public_root(&self) -> Result<PathBuf, String> {
        let public_root = self.root.join("resources");
        let resolved = public_root.canonicalize().map_err(|error| {
            format!(
                "cannot resolve desktop resources directory {}: {error}",
                public_root.display()
            )
        })?;
        if !resolved.is_dir() || !resolved.starts_with(&self.root) {
            return Err(format!(
                "desktop resources directory is invalid: {}",
                resolved.display()
            ));
        }
        Ok(resolved)
    }

    pub fn resolve_entry(&self, entry: &str) -> Result<PathBuf, String> {
        let relative = Path::new(entry);
        if relative.is_absolute()
            || relative
                .components()
                .any(|component| !matches!(component, Component::Normal(_) | Component::CurDir))
        {
            return Err("desktop entry must be a relative path inside the project".to_owned());
        }

        let resolved = self.root.join(relative).canonicalize().map_err(|error| {
            format!(
                "cannot resolve desktop entry {}: {error}",
                self.root.join(relative).display()
            )
        })?;
        let public_root = self.public_root()?;
        if !resolved.starts_with(&public_root) || !resolved.is_file() {
            return Err(format!(
                "desktop entry must be a file inside {}: {}",
                public_root.display(),
                resolved.display()
            ));
        }
        Ok(resolved)
    }

    pub fn validate_bootstrap(&self, bootstrap: &Bootstrap) -> Result<(), String> {
        bootstrap.validate()?;
        for window in &bootstrap.windows {
            self.resolve_entry(&window.entry)?;
        }
        Ok(())
    }
}
