use std::path::{Component, Path, PathBuf};

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
        if !resolved.starts_with(&self.root) || !resolved.is_file() {
            return Err(format!(
                "desktop entry escapes the project or is not a file: {}",
                resolved.display()
            ));
        }
        Ok(resolved)
    }
}
