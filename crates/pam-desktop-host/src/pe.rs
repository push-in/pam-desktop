//! The DLLs a Windows executable or library imports, read from its PE
//! import and delay-load tables, so a bundle ships the libraries beside PHP
//! that it and its extensions really load instead of every DLL of the
//! archive (the official PHP for Windows carries glib, enchant, libpq,
//! phpdbg... beside `php.exe`).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

fn u16_at(bytes: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        bytes.get(offset..offset + 2)?.try_into().ok()?,
    ))
}

fn u32_at(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        bytes.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

/// The names of the DLLs `image` imports, lowercase; `None` when it is not
/// a PE image this can read.
pub fn imports(image: &[u8]) -> Option<BTreeSet<String>> {
    if image.get(..2)? != b"MZ" {
        return None;
    }
    let pe = u32_at(image, 0x3C)? as usize;
    if image.get(pe..pe + 4)? != b"PE\0\0" {
        return None;
    }
    let coff = pe + 4;
    let sections = u16_at(image, coff + 2)? as usize;
    let optional_size = u16_at(image, coff + 16)? as usize;
    let optional = coff + 20;
    let directories = match u16_at(image, optional)? {
        0x10B => optional + 96,
        0x20B => optional + 112,
        _ => return None,
    };
    let directory_count = u32_at(image, directories - 4)? as usize;
    let table = optional + optional_size;
    let to_offset = |rva: u32| -> Option<usize> {
        (0..sections.min(96)).find_map(|index| {
            let header = table + index * 40;
            let virtual_size = u32_at(image, header + 8)?;
            let address = u32_at(image, header + 12)?;
            let raw_size = u32_at(image, header + 16)?;
            let raw = u32_at(image, header + 20)?;
            let extent = virtual_size.max(raw_size);
            // Checked: a malformed image must not overflow (a panic in debug builds).
            (rva >= address && rva < address.checked_add(extent)?)
                .then(|| (rva - address).checked_add(raw))
                .flatten()
                .map(|offset| offset as usize)
        })
    };
    let name_at = |rva: u32| -> Option<String> {
        let start = to_offset(rva)?;
        let rest = image.get(start..)?;
        let end = rest.iter().take(260).position(|byte| *byte == 0)?;
        let name = std::str::from_utf8(&rest[..end]).ok()?;
        (!name.is_empty()).then(|| name.to_ascii_lowercase())
    };
    let mut names = BTreeSet::new();
    // 1: the import table (20-byte descriptors, name at 12); 13: the
    // delay-load table (32-byte descriptors, name at 4).
    for (directory, size, name_field) in [(1_usize, 20_usize, 12_usize), (13, 32, 4)] {
        if directory >= directory_count {
            continue;
        }
        let rva = u32_at(image, directories + directory * 8)?;
        if rva == 0 {
            continue;
        }
        let Some(mut descriptor) = to_offset(rva) else {
            continue;
        };
        for _ in 0..4096 {
            let Some(entry) = image.get(descriptor..descriptor + size) else {
                break;
            };
            if entry.iter().all(|byte| *byte == 0) {
                break;
            }
            if let Some(name) = u32_at(image, descriptor + name_field).and_then(name_at) {
                names.insert(name);
            }
            descriptor += size;
        }
    }
    Some(names)
}

/// The DLLs of `directory` that `roots` load, directly or through one
/// another; `None` when one of the roots cannot be read as a PE image.
pub fn needed_libraries(directory: &Path, roots: &[&Path]) -> Option<Vec<PathBuf>> {
    Some(resolve(directory, roots)?.0)
}

/// The Visual C++ runtime DLLs the images load that are not beside them:
/// `vcruntime140.dll` and friends come with the Visual C++ Redistributable,
/// which a clean Windows does not have.
pub fn missing_vc_runtime(directory: &Path, roots: &[&Path]) -> BTreeSet<String> {
    resolve(directory, roots)
        .map(|(_, missing)| {
            missing
                .into_iter()
                .filter(|name| is_vc_runtime(name))
                .collect()
        })
        .unwrap_or_default()
}

/// Whether one of the images loads `name` and it is not beside them.
pub fn misses(directory: &Path, roots: &[&Path], name: &str) -> bool {
    resolve(directory, roots)
        .is_some_and(|(_, missing)| missing.contains(&name.to_ascii_lowercase()))
}

/// `vcruntime140.dll`, `vcruntime140_1.dll`, `msvcp140.dll`… (app-local
/// deployment of these is allowed by the Visual C++ Redistributable).
pub fn is_vc_runtime(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    (name.starts_with("vcruntime140")
        || name.starts_with("msvcp140")
        || name.starts_with("concrt140"))
        && name.ends_with(".dll")
}

fn resolve(directory: &Path, roots: &[&Path]) -> Option<(Vec<PathBuf>, BTreeSet<String>)> {
    let available: BTreeMap<String, PathBuf> = fs::read_dir(directory)
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?.to_ascii_lowercase();
            (name.ends_with(".dll") && entry.path().is_file()).then(|| (name, entry.path()))
        })
        .collect();
    let mut needed = BTreeSet::new();
    let mut missing = BTreeSet::new();
    let mut pending: Vec<PathBuf> = roots.iter().map(|root| root.to_path_buf()).collect();
    while let Some(path) = pending.pop() {
        let image = fs::read(&path).ok();
        let Some(names) = image.as_deref().and_then(imports) else {
            // A library that cannot be read is still shipped; a root that
            // cannot be read means nothing can be decided.
            if roots.iter().any(|root| *root == path) {
                return None;
            }
            continue;
        };
        for name in names {
            match available.get(&name) {
                Some(library) => {
                    if needed.insert(name) {
                        pending.push(library.clone());
                    }
                }
                None => {
                    missing.insert(name);
                }
            }
        }
    }
    let needed = needed
        .into_iter()
        .filter_map(|name| available.get(&name).cloned())
        .collect();
    Some((needed, missing))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal PE32+ image with one section holding an import table that
    /// names `names`, and a delay-load table that names `delayed`.
    fn image(names: &[&str], delayed: &[&str]) -> Vec<u8> {
        let mut bytes = vec![0_u8; 0x400];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[0x3C..0x40].copy_from_slice(&0x80_u32.to_le_bytes());
        bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
        let coff = 0x84;
        bytes[coff + 2..coff + 4].copy_from_slice(&1_u16.to_le_bytes());
        bytes[coff + 16..coff + 18].copy_from_slice(&240_u16.to_le_bytes());
        let optional = coff + 20;
        bytes[optional..optional + 2].copy_from_slice(&0x20B_u16.to_le_bytes());
        let directories = optional + 112;
        bytes[directories - 4..directories].copy_from_slice(&16_u32.to_le_bytes());
        // One section: RVA 0x1000 maps to file offset 0x200.
        let section = optional + 240;
        bytes[section + 8..section + 12].copy_from_slice(&0x200_u32.to_le_bytes());
        bytes[section + 12..section + 16].copy_from_slice(&0x1000_u32.to_le_bytes());
        bytes[section + 16..section + 20].copy_from_slice(&0x200_u32.to_le_bytes());
        bytes[section + 20..section + 24].copy_from_slice(&0x200_u32.to_le_bytes());
        let mut strings = 0x300;
        let mut put_name = |bytes: &mut Vec<u8>, name: &str| {
            let rva = 0x1000 + (strings - 0x200) as u32;
            bytes[strings..strings + name.len()].copy_from_slice(name.as_bytes());
            strings += name.len() + 1;
            rva
        };
        let imports_at = 0x200;
        bytes[directories + 8..directories + 12].copy_from_slice(&0x1000_u32.to_le_bytes());
        for (index, name) in names.iter().enumerate() {
            let rva = put_name(&mut bytes, name);
            let descriptor = imports_at + index * 20;
            bytes[descriptor + 12..descriptor + 16].copy_from_slice(&rva.to_le_bytes());
        }
        let delayed_at = 0x280;
        bytes[directories + 13 * 8..directories + 13 * 8 + 4]
            .copy_from_slice(&(0x1080_u32).to_le_bytes());
        for (index, name) in delayed.iter().enumerate() {
            let rva = put_name(&mut bytes, name);
            let descriptor = delayed_at + index * 32;
            bytes[descriptor..descriptor + 4].copy_from_slice(&1_u32.to_le_bytes());
            bytes[descriptor + 4..descriptor + 8].copy_from_slice(&rva.to_le_bytes());
        }
        bytes
    }

    #[test]
    fn reads_imports_and_delay_loads() {
        let names = imports(&image(&["php8.dll", "KERNEL32.dll"], &["libssl-3-x64.dll"])).unwrap();
        assert_eq!(
            names.into_iter().collect::<Vec<_>>(),
            ["kernel32.dll", "libssl-3-x64.dll", "php8.dll"]
        );
        assert!(imports(b"#!/bin/sh\n").is_none());
        assert!(imports(&[]).is_none());
    }

    #[test]
    fn follows_only_the_libraries_that_are_loaded() {
        let directory = std::env::temp_dir().join(format!("pam-pe-imports-{}", std::process::id()));
        let ext = directory.join("ext");
        fs::create_dir_all(&ext).unwrap();
        fs::write(
            directory.join("php.exe"),
            image(&["php8.dll", "kernel32.dll"], &[]),
        )
        .unwrap();
        fs::write(
            directory.join("php8.dll"),
            image(&["libcrypto-3-x64.dll"], &[]),
        )
        .unwrap();
        fs::write(
            directory.join("libcrypto-3-x64.dll"),
            image(&["kernel32.dll"], &[]),
        )
        .unwrap();
        fs::write(directory.join("ICUUC77.dll"), image(&["icudt77.dll"], &[])).unwrap();
        fs::write(directory.join("icudt77.dll"), image(&[], &[])).unwrap();
        fs::write(directory.join("glib-2.dll"), image(&[], &[])).unwrap();
        fs::write(
            ext.join("php_intl.dll"),
            image(&["php8.dll", "icuuc77.dll"], &[]),
        )
        .unwrap();
        let names = |found: Vec<PathBuf>| {
            found
                .iter()
                .map(|path| path.file_name().unwrap().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
        };
        let php = directory.join("php.exe");
        assert_eq!(
            names(needed_libraries(&directory, &[&php]).unwrap()),
            ["libcrypto-3-x64.dll", "php8.dll"]
        );
        let intl = ext.join("php_intl.dll");
        assert_eq!(
            names(needed_libraries(&directory, &[&php, &intl]).unwrap()),
            [
                "icudt77.dll",
                "ICUUC77.dll",
                "libcrypto-3-x64.dll",
                "php8.dll"
            ]
        );
        fs::write(
            directory.join("tool.exe"),
            image(&["VCRUNTIME140.dll", "kernel32.dll"], &[]),
        )
        .unwrap();
        assert_eq!(
            missing_vc_runtime(&directory, &[&directory.join("tool.exe"), &php]),
            BTreeSet::from(["vcruntime140.dll".to_owned()])
        );
        fs::write(directory.join("broken.exe"), b"#!/bin/sh\n").unwrap();
        assert!(needed_libraries(&directory, &[&directory.join("broken.exe")]).is_none());
        fs::remove_dir_all(directory).unwrap();
    }
}
