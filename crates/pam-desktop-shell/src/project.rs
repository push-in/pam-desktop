use std::fs;
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
        let resolved = self.resolve_public_asset(entry, "desktop entry")?;
        if !resolved.is_file() {
            return Err(format!(
                "desktop entry is not a file: {}",
                resolved.display()
            ));
        }
        Ok(resolved)
    }

    pub fn resolve_icon(&self, icon: &str) -> Result<PathBuf, String> {
        const MAX_ICON_BYTES: u64 = 2 * 1024 * 1024;

        let resolved = self.resolve_public_asset(icon, "application icon")?;
        let metadata = resolved
            .metadata()
            .map_err(|error| format!("cannot inspect icon {}: {error}", resolved.display()))?;
        if !metadata.is_file() || metadata.len() > MAX_ICON_BYTES {
            return Err(format!(
                "application icon must be a file no larger than {MAX_ICON_BYTES} bytes: {}",
                resolved.display()
            ));
        }
        let contents = fs::read(&resolved)
            .map_err(|error| format!("cannot read icon {}: {error}", resolved.display()))?;
        match resolved
            .extension()
            .and_then(|extension| extension.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("png") => validate_png_icon(&contents)?,
            Some("svg") => validate_svg_icon(&contents)?,
            _ => return Err("application icon must use the PNG or SVG extension".to_owned()),
        }
        Ok(resolved)
    }

    fn resolve_public_asset(&self, asset: &str, label: &str) -> Result<PathBuf, String> {
        let relative = Path::new(asset);
        if relative.is_absolute()
            || relative
                .components()
                .any(|component| !matches!(component, Component::Normal(_) | Component::CurDir))
        {
            return Err(format!(
                "{label} must be a relative path inside the project"
            ));
        }

        let resolved = self.root.join(relative).canonicalize().map_err(|error| {
            format!(
                "cannot resolve {label} {}: {error}",
                self.root.join(relative).display()
            )
        })?;
        let public_root = self.public_root()?;
        if !resolved.starts_with(&public_root) {
            return Err(format!(
                "{label} must be inside {}: {}",
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
        self.resolve_icon(&bootstrap.manifest.icon)?;
        Ok(())
    }
}

fn validate_png_icon(contents: &[u8]) -> Result<(), String> {
    const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    if contents.len() < 24
        || contents.get(..8) != Some(PNG_SIGNATURE)
        || contents.get(12..16) != Some(b"IHDR")
    {
        return Err("application PNG icon has an invalid header".to_owned());
    }
    let width = u32::from_be_bytes(
        contents[16..20]
            .try_into()
            .expect("the PNG header length was checked"),
    );
    let height = u32::from_be_bytes(
        contents[20..24]
            .try_into()
            .expect("the PNG header length was checked"),
    );
    if width != height || !(64..=1024).contains(&width) {
        return Err("application PNG icon must be square and between 64px and 1024px".to_owned());
    }
    Ok(())
}

fn validate_svg_icon(contents: &[u8]) -> Result<(), String> {
    let source = std::str::from_utf8(contents)
        .map_err(|_| "application SVG icon must contain valid UTF-8".to_owned())?;
    let normalized = source.to_ascii_lowercase();
    if !normalized.contains("<svg")
        || normalized.contains("<script")
        || normalized.contains("<!doctype")
        || normalized.contains("href=\"http")
        || normalized.contains("href='http")
    {
        return Err(
            "application SVG icon must be self-contained and cannot include scripts or remote resources"
                .to_owned(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_self_contained_application_icons() {
        assert!(
            validate_svg_icon(
                b"<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 64 64\"></svg>"
            )
            .is_ok()
        );
        assert!(validate_svg_icon(b"<svg><script>alert(1)</script></svg>").is_err());
        assert!(
            validate_svg_icon(b"<svg><image href=\"https://example.com/icon.png\"/></svg>")
                .is_err()
        );

        let mut png = Vec::from(*b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0@\0\0\0@");
        assert!(validate_png_icon(&png).is_ok());
        png[23] = 32;
        assert!(validate_png_icon(&png).is_err());
    }
}
