use std::ffi::OsString;
use std::fs;
use std::io::Cursor;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::project::Project;

const MAX_PNG_BYTES: u64 = 32 * 1024 * 1024;
const MAX_DECODED_BYTES: usize = 64 * 1024 * 1024;
const MAX_DIMENSION: u32 = 8_192;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Action {
    Accept,
    Verify,
}

#[derive(Debug)]
struct Options {
    action: Action,
    project: PathBuf,
    name: String,
    actual: PathBuf,
    force: bool,
}

#[derive(Debug, Eq, PartialEq)]
struct Image {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Evidence<'a> {
    schema_version: u8,
    surface_code: u8,
    comparison_code: u8,
    case: &'a str,
    width: u32,
    height: u32,
    changed_pixels: u64,
    actual_sha256: String,
    golden_sha256: String,
    project_revision: Option<String>,
    host_os: &'static str,
    host_arch: &'static str,
    pam_desktop_version: &'static str,
}

pub fn run(arguments: Vec<OsString>) -> Result<(), String> {
    let options = parse(arguments)?;
    let project = Project::discover(&options.project)?;
    let actual_path = resolve_project_file(project.root(), &options.actual, "actual screenshot")?;
    let actual_bytes = read_png(&actual_path)?;
    let actual = decode_png(&actual_bytes, &actual_path)?;
    let golden_path = project
        .root()
        .join("tests")
        .join("visual")
        .join(format!("{}.png", options.name));

    match options.action {
        Action::Accept => {
            if golden_path.exists() && !options.force {
                return Err(format!(
                    "visual golden already exists at {}; pass --force to replace it",
                    golden_path.display()
                ));
            }
            write_scoped(project.root(), &golden_path, &actual_bytes)?;
            println!("[ok] Accepted visual golden: {}", golden_path.display());
            println!(
                "[next] Verify it with: pam-desktop visual verify --name {} --actual {}",
                options.name,
                options.actual.display()
            );
            Ok(())
        }
        Action::Verify => verify(
            &project,
            &options.name,
            &actual_bytes,
            &actual,
            &golden_path,
        ),
    }
}

fn verify(
    project: &Project,
    name: &str,
    actual_bytes: &[u8],
    actual: &Image,
    golden_path: &Path,
) -> Result<(), String> {
    let resolved_golden = golden_path.canonicalize().map_err(|error| {
        format!(
            "cannot resolve visual golden {}: {error}",
            golden_path.display()
        )
    })?;
    if !resolved_golden.starts_with(project.root()) || !resolved_golden.is_file() {
        return Err("visual golden must be a regular file inside the project".to_owned());
    }
    let golden_bytes = read_png(&resolved_golden)?;
    let golden = decode_png(&golden_bytes, &resolved_golden)?;
    let dimensions_match = actual.width == golden.width && actual.height == golden.height;
    let changed_pixels = if dimensions_match {
        actual
            .rgba
            .chunks_exact(4)
            .zip(golden.rgba.chunks_exact(4))
            .filter(|(left, right)| left != right)
            .count() as u64
    } else {
        u64::from(actual.width) * u64::from(actual.height)
            + u64::from(golden.width) * u64::from(golden.height)
    };
    let matches = dimensions_match && changed_pixels == 0;
    let evidence = Evidence {
        schema_version: 1,
        surface_code: 3,
        comparison_code: if matches { 1 } else { 2 },
        case: name,
        width: actual.width,
        height: actual.height,
        changed_pixels,
        actual_sha256: digest(actual_bytes),
        golden_sha256: digest(&golden_bytes),
        project_revision: project_revision(project.root()),
        host_os: std::env::consts::OS,
        host_arch: std::env::consts::ARCH,
        pam_desktop_version: env!("CARGO_PKG_VERSION"),
    };
    let encoded = serde_json::to_vec_pretty(&evidence)
        .map_err(|error| format!("cannot encode visual evidence: {error}"))?;
    let evidence_path = project
        .root()
        .join("artifacts")
        .join("visual")
        .join(format!("{name}.json"));
    write_scoped(project.root(), &evidence_path, &encoded)?;

    if !matches {
        return Err(format!(
            "visual case {name:?} differs from its golden: {changed_pixels} changed pixels; evidence: {}",
            evidence_path.display()
        ));
    }
    println!(
        "[ok] Visual case {name}: {}x{}, exact pixel match",
        actual.width, actual.height
    );
    println!("[ok] Evidence: {}", evidence_path.display());
    Ok(())
}

fn parse(arguments: Vec<OsString>) -> Result<Options, String> {
    let mut arguments = arguments.into_iter();
    let action = match arguments
        .next()
        .and_then(|value| value.into_string().ok())
        .as_deref()
    {
        Some("accept") => Action::Accept,
        Some("verify") => Action::Verify,
        _ => return Err(usage()),
    };
    let mut project = PathBuf::from(".");
    let mut name = None;
    let mut actual = None;
    let mut force = false;
    let mut positional = false;
    while let Some(argument) = arguments.next() {
        match argument.to_str() {
            Some("--name") => name = Some(next_utf8(&mut arguments, "--name")?),
            Some("--actual") => {
                actual = Some(PathBuf::from(next_utf8(&mut arguments, "--actual")?));
            }
            Some("--force") if action == Action::Accept => force = true,
            Some(value) if !value.starts_with('-') && !positional => {
                project = PathBuf::from(value);
                positional = true;
            }
            _ => return Err(usage()),
        }
    }
    let name = name.ok_or_else(usage)?;
    if !valid_name(&name) {
        return Err(
            "visual case names must be 1-64 lowercase letters, digits, dots or hyphens".to_owned(),
        );
    }
    let actual = actual.ok_or_else(usage)?;
    validate_relative(&actual, "actual screenshot")?;
    Ok(Options {
        action,
        project,
        name,
        actual,
        force,
    })
}

fn next_utf8(arguments: &mut impl Iterator<Item = OsString>, flag: &str) -> Result<String, String> {
    arguments
        .next()
        .ok_or_else(usage)?
        .into_string()
        .map_err(|_| format!("{flag} must contain valid UTF-8"))
}

fn usage() -> String {
    "usage: pam-desktop visual accept|verify [directory] --name <case> --actual <project-relative.png> [--force]".to_owned()
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'-')
        })
        && !name.starts_with('.')
        && !name.ends_with('.')
}

fn validate_relative(path: &Path, label: &str) -> Result<(), String> {
    if path.extension().and_then(|value| value.to_str()) != Some("png")
        || path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!(
            "{label} must be a project-relative .png path without traversal"
        ));
    }
    Ok(())
}

fn resolve_project_file(root: &Path, relative: &Path, label: &str) -> Result<PathBuf, String> {
    validate_relative(relative, label)?;
    let resolved = root.join(relative).canonicalize().map_err(|error| {
        format!(
            "cannot resolve {label} {}: {error}",
            root.join(relative).display()
        )
    })?;
    if !resolved.starts_with(root) || !resolved.is_file() {
        return Err(format!("{label} must be a regular file inside the project"));
    }
    Ok(resolved)
}

fn read_png(path: &Path) -> Result<Vec<u8>, String> {
    let metadata = path
        .metadata()
        .map_err(|error| format!("cannot inspect PNG {}: {error}", path.display()))?;
    if !metadata.is_file() || metadata.len() > MAX_PNG_BYTES {
        return Err(format!(
            "PNG must be a regular file no larger than {MAX_PNG_BYTES} bytes: {}",
            path.display()
        ));
    }
    fs::read(path).map_err(|error| format!("cannot read PNG {}: {error}", path.display()))
}

fn decode_png(bytes: &[u8], path: &Path) -> Result<Image, String> {
    let limits = png::Limits {
        bytes: MAX_DECODED_BYTES,
    };
    let mut decoder = png::Decoder::new_with_limits(Cursor::new(bytes), limits);
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder
        .read_info()
        .map_err(|error| format!("invalid PNG {}: {error}", path.display()))?;
    let size = reader
        .output_buffer_size()
        .ok_or_else(|| format!("PNG dimensions overflow at {}", path.display()))?;
    if size > MAX_DECODED_BYTES {
        return Err(format!(
            "decoded PNG exceeds {MAX_DECODED_BYTES} bytes: {}",
            path.display()
        ));
    }
    let mut pixels = vec![0; size];
    let info = reader
        .next_frame(&mut pixels)
        .map_err(|error| format!("cannot decode PNG {}: {error}", path.display()))?;
    if info.width == 0
        || info.height == 0
        || info.width > MAX_DIMENSION
        || info.height > MAX_DIMENSION
    {
        return Err(format!(
            "PNG dimensions must be between 1 and {MAX_DIMENSION}px: {}",
            path.display()
        ));
    }
    pixels.truncate(info.buffer_size());
    let rgba = to_rgba(&pixels, info.color_type)?;
    Ok(Image {
        width: info.width,
        height: info.height,
        rgba,
    })
}

fn to_rgba(pixels: &[u8], color: png::ColorType) -> Result<Vec<u8>, String> {
    let mut rgba = Vec::with_capacity(pixels.len().saturating_mul(4));
    match color {
        png::ColorType::Rgba => return Ok(pixels.to_vec()),
        png::ColorType::Rgb => pixels
            .chunks_exact(3)
            .for_each(|pixel| rgba.extend_from_slice(&[pixel[0], pixel[1], pixel[2], 255])),
        png::ColorType::Grayscale => pixels
            .iter()
            .for_each(|value| rgba.extend_from_slice(&[*value, *value, *value, 255])),
        png::ColorType::GrayscaleAlpha => pixels
            .chunks_exact(2)
            .for_each(|pixel| rgba.extend_from_slice(&[pixel[0], pixel[0], pixel[0], pixel[1]])),
        png::ColorType::Indexed => {
            return Err("indexed PNG was not expanded by the decoder".to_owned());
        }
    }
    Ok(rgba)
}

fn write_scoped(root: &Path, destination: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = destination
        .parent()
        .ok_or_else(|| "visual output has no parent".to_owned())?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    let resolved_parent = parent
        .canonicalize()
        .map_err(|error| format!("cannot resolve {}: {error}", parent.display()))?;
    if !resolved_parent.starts_with(root) {
        return Err("visual output escapes the project through a symbolic link".to_owned());
    }
    if destination
        .symlink_metadata()
        .is_ok_and(|metadata| metadata.file_type().is_symlink() || metadata.is_dir())
    {
        return Err(format!(
            "visual output is not a regular project file: {}",
            destination.display()
        ));
    }
    let temporary = parent.join(format!(".pam-visual-{}.tmp", std::process::id()));
    let result = (|| {
        fs::write(&temporary, bytes)
            .map_err(|error| format!("cannot write {}: {error}", temporary.display()))?;
        if destination.is_file() {
            fs::remove_file(destination)
                .map_err(|error| format!("cannot replace {}: {error}", destination.display()))?;
        }
        fs::rename(&temporary, destination)
            .map_err(|error| format!("cannot replace {}: {error}", destination.display()))
    })();
    let _ = fs::remove_file(&temporary);
    result
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn project_revision(root: &Path) -> Option<String> {
    let output = Command::new("git")
        .args(["-C"])
        .arg(root)
        .args(["rev-parse", "--verify", "HEAD"])
        .output()
        .ok()?;
    let revision = String::from_utf8(output.stdout).ok()?;
    let revision = revision.trim();
    (output.status.success()
        && revision.len() == 40
        && revision.bytes().all(|byte| byte.is_ascii_hexdigit()))
    .then(|| revision.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_root() -> PathBuf {
        std::env::temp_dir().join(format!(
            "pam-desktop-visual-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ))
    }

    fn png(color: [u8; 4]) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut encoder = png::Encoder::new(&mut bytes, 1, 1);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().expect("header");
        writer.write_image_data(&color).expect("pixel");
        writer.finish().expect("finish");
        bytes
    }

    #[test]
    fn decodes_pixels_instead_of_comparing_png_container_bytes() {
        let bytes = png([1, 2, 3, 255]);
        let image = decode_png(&bytes, Path::new("fixture.png")).expect("valid PNG");
        assert_eq!(
            image,
            Image {
                width: 1,
                height: 1,
                rgba: vec![1, 2, 3, 255]
            }
        );
        assert!(decode_png(b"not png", Path::new("fixture.png")).is_err());
    }

    #[test]
    fn confines_visual_names_and_paths() {
        assert!(valid_name("settings.dark"));
        assert!(!valid_name("../escape"));
        assert!(!valid_name("Settings"));
        assert!(validate_relative(Path::new("artifacts/desktop.png"), "actual").is_ok());
        assert!(validate_relative(Path::new("../desktop.png"), "actual").is_err());
        assert!(validate_relative(Path::new("desktop.jpg"), "actual").is_err());
    }

    #[test]
    fn accepts_verifies_and_records_visual_regressions() {
        let root = fixture_root();
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("vendor")).expect("vendor");
        fs::create_dir_all(root.join("artifacts/screenshots")).expect("screenshots");
        fs::write(root.join("app.php"), "<?php").expect("app");
        fs::write(root.join("composer.json"), "{}").expect("composer");
        fs::write(root.join("vendor/autoload.php"), "<?php").expect("autoload");
        let actual = root.join("artifacts/screenshots/main.png");
        fs::write(&actual, png([1, 2, 3, 255])).expect("actual");

        run(vec![
            "accept".into(),
            root.as_os_str().into(),
            "--name".into(),
            "main.dark".into(),
            "--actual".into(),
            "artifacts/screenshots/main.png".into(),
        ])
        .expect("accept");
        run(vec![
            "verify".into(),
            root.as_os_str().into(),
            "--name".into(),
            "main.dark".into(),
            "--actual".into(),
            "artifacts/screenshots/main.png".into(),
        ])
        .expect("verify");

        fs::write(&actual, png([4, 5, 6, 255])).expect("changed actual");
        let error = run(vec![
            "verify".into(),
            root.as_os_str().into(),
            "--name".into(),
            "main.dark".into(),
            "--actual".into(),
            "artifacts/screenshots/main.png".into(),
        ])
        .expect_err("regression");
        assert!(error.contains("1 changed pixels"));

        let evidence: serde_json::Value = serde_json::from_slice(
            &fs::read(root.join("artifacts/visual/main.dark.json")).expect("evidence"),
        )
        .expect("JSON");
        assert_eq!(evidence["schemaVersion"], 1);
        assert_eq!(evidence["surfaceCode"], 3);
        assert_eq!(evidence["comparisonCode"], 2);
        assert_eq!(evidence["changedPixels"], 1);
        fs::remove_dir_all(root).expect("cleanup");
    }
}
