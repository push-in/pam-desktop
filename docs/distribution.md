# Desktop distribution

`pam-desktop build` turns an application into a self-contained bundle for the
system the build runs on, and into the installers of that system. The bundle
carries the native host, the application, a PHP runtime and a launcher: the
machine it is installed on needs neither PHP nor Composer.

The identity of the application comes from `pam-desktop.json`. Nothing is
booted and no bundler is involved: the build reads the manifest, copies files
and calls the packaging tools of the system.

## Application manifest

```json
{
    "applicationId": "com.example.my-app",
    "name": "My application",
    "version": "1.0.0",
    "build": 42,
    "entry": "index.php",
    "description": "A focused desktop tool written in PHP.",
    "publisher": "Example",
    "category": "productivity",
    "icon": "resources/icon.png",
    "exclude": ["storage/cache", "storage/development.sqlite"],
    "extensions": ["ctype", "mbstring", "tokenizer"],
    "urlSchemes": ["my-app"],
    "singleInstance": true,
    "window": { "width": 1100, "height": 720 }
}
```

Every distribution field is optional:

| Field | Default | Rules |
| --- | --- | --- |
| `applicationId` | `dev.pam.desktop.app-<folder>` | lowercase reverse-DNS name of at most 155 bytes; each segment starts with a letter and holds letters, digits or dashes |
| `name` | the folder name | 1 to 80 bytes of printable text |
| `version` | `0.0.0` | starts with a digit; at most 64 letters, digits, `.`, `+`, `-`, `~` |
| `build` | none | at most 64 bytes of printable text |
| `entry` | `index.php` | the script `run`, `dev` and the bundle start; a path inside the project |
| `description` | empty | at most 256 bytes of printable text |
| `publisher` | `name` | 1 to 128 bytes of printable text |
| `category` | `utility` | `development`, `productivity`, `graphics`, `audio-video`, `network`, `utility`, `game`, `education` |
| `icon` | a generic icon | project-relative PNG (square, 64 to 1024 px) or self-contained SVG, at most 2 MiB |
| `exclude` | nothing | project-relative paths left out of the bundle |
| `extensions` | every shared extension PHP loads on the build machine | lowercase PHP extension names to ship instead |
| `urlSchemes` | none | up to 8 schemes of the links that open the application |
| `singleInstance` | `true` when `urlSchemes` is not empty (never by default under `dev`) | `true` or `false` |
| `updates` | none | see [Signed updates](updates.md) |

`pam-desktop run` and `pam-desktop dev` are lenient: a missing or malformed
manifest means defaults. `pam-desktop build` and `pam-desktop publish-update`
are strict: a field with the wrong type or an unsafe value stops the command
with the reason. An application that declares no `applicationId` is built
with the default above; declare one before distributing, because it names the
package, the desktop entry, the data directories and the update feed.

The paths the application cannot start without (`vendor`, `composer.json`,
`composer.lock`, `pam-desktop.json`, the entry script and the directories that
hold them) cannot be excluded. The build also leaves out `.git`, `.pam`,
`.pam-desktop` (the state of development runs), `.pamignore`, `.env`, `.env.*`,
`dist`, `node_modules`, `target`, the output directory itself and the nested
`vendor` directories of vendored packages. `type=path` repositories and the
local paths Composer recorded for them are removed from the bundled
`composer.json` and `composer.lock`.

A bundle holds no symbolic links. A link is replaced by a copy of what it
points at; a link that leaves the project is only accepted below `vendor`
(where Composer links path packages), a link into the output directory and a
cycle of links stop the build.

### Links and single instance

Each scheme of `urlSchemes` is a lowercase letter followed by 1 to 31 lowercase
letters, digits, `+`, `.` or `-`; `http`, `https`, `file`, `mailto`,
`javascript` and `data` cannot be claimed. The installers register them:

| System | Registration |
| --- | --- |
| Linux | `MimeType=x-scheme-handler/<scheme>;` in the desktop entry (`.deb`, `.rpm`, AppImage, `install.sh`), which starts the launcher with `%u` |
| macOS | `CFBundleURLTypes` in the `Info.plist` of the `.app` |
| Windows | `windows.protocol` extensions in the MSIX; per-user `Software\Classes\<scheme>` registry keys in the MSI |

The Windows portable `.zip` installs nothing. To open links with it, create
the keys the MSI writes: `HKCU\Software\Classes\<scheme>` with the values
`(Default) = URL:<name>` and `URL Protocol = ""`, and
`shell\open\command` below it with
`(Default) = "<bundle>\bin\<name>.exe" "%1"`.

The launcher hands the link it was started with to the host as
`PAM_DESKTOP_LAUNCH_URL`; `pam-desktop run <app> <url>` does the same from a
terminal. With `singleInstance`, a second launch passes its link to the
running application and exits.

## Build formats

```bash
# Bundle directory + update-ready portable archive
pam-desktop build

pam-desktop build path/to/app --format deb
pam-desktop build --format appimage --format rpm
pam-desktop build --output artifacts --format all
```

`build [app]` takes a project directory (the current one by default) or a
script, which then becomes the entry of the bundle. Artifacts are written to
`--output`, by default `dist` inside the project.

Existing destinations cause a failure. Pass `--force` only to replace those
exact versioned artifacts.

| Host | `directory` | `portable` | installers |
| --- | --- | --- | --- |
| Linux | self-contained directory | deterministic `.tar.gz` | `appimage`, `deb`, `rpm` |
| macOS | self-contained directory | `.zip` of the `.app` | `dmg`, `pkg` |
| Windows | self-contained directory | `.zip` | `msix`, `msi` |

Each installer is built on its target system; `--format all` selects every
format of the host. The tools a requested format needs are looked up on `PATH`
before any work starts: `tar` and `gzip`, `dpkg-deb`, the official
`appimagetool`, `rpmbuild`, `ditto`, `hdiutil`, `pkgbuild`, `powershell`,
`makeappx`, WiX 5 (`wix`). Every artifact is staged in a private directory
inside the output and published by renaming only after all of them were
built, so a missing or failing tool never leaves an incomplete artifact.

## The bundle

```text
<name>-<version>-<os>-<arch>/
  manifest.json            identity, target and SHA-256 of every file
  bin/pam-desktop          the native host
  bin/<name>               the launcher
  bin/pam-desktop-php      starts the bundled PHP (Linux, macOS)
  runtime/php/bin/php      the PHP runtime
  runtime/php/lib/         the shared libraries PHP and its extensions need
  runtime/php/ext/         the shared PHP extensions
  etc/php.ini              expose_php=Off, display_errors=Off, log_errors=On,
                           memory_limit=512M and the shipped extensions
  lib/                     libraries of the host the system does not provide
  app/                     the application, with its vendor directory
  share/                   desktop entry template and icon (Linux)
  install.sh uninstall.sh  per-user installation (Linux)
```

`<name>` is the last segment of `applicationId`.

**The host** is the `pam-desktop` binary that runs the build, so build with
the host you mean to ship.

**PHP** is found the way `run` finds it: `PAM_DESKTOP_PHP`, then `pam` on
`PATH` (started as `pam exec`), then `php` on `PATH`. The build asks it for its
version, its extension directory and the extensions it loads, and ships the
shared ones, or only those of `extensions`. An extension compiled into the
binary needs nothing; one that is listed and is neither loaded nor present in
the extension directory stops the build. Extensions are loaded from the
bundled `php.ini` only: the configuration of the build machine is not shipped,
and the configuration of the machine the application runs on is not read
(`PHPRC` points at the bundle and `PHP_INI_SCAN_DIR` is empty). On Linux the
shared libraries of the binary and of each extension are collected with `ldd`
and bundled, except the C library itself. On macOS and Windows the libraries
that sit beside the binary are bundled: give the build a relocatable PHP, such
as a static build or the official Windows archive.

**The launcher** starts `pam-desktop run` on the bundled project with
`PAM_DESKTOP_PHP` pointing at the bundled PHP, and exports
`PAM_DESKTOP_BUNDLE_ROOT` and `PAM_DESKTOP_LAUNCHER` for the updater. On Linux
and macOS it is a shell script that resolves links to itself; PHP's own
libraries and configuration are set by `bin/pam-desktop-php`, for the PHP
process alone. On Windows the launcher is the host under the name of the
application: started from `bin` of a bundle under a name other than
`pam-desktop`, the host runs the bundle.

**The system provides the desktop platform.** On Linux the host uses the GTK 4
of the machine it runs on, with WebKitGTK 6 and GStreamer when it was built
with them, so that themes, portals and GPU drivers match. These libraries and
everything they depend on are not bundled; `.deb` and `.rpm` packages declare
them as dependencies (`libgtk-4-1`, `libwebkitgtk-6.0-4`,
`gstreamer1.0-plugins-base`, `gstreamer1.0-plugins-good`; `gtk4`,
`webkitgtk6.0`, `gstreamer1-plugins-base`, `gstreamer1-plugins-good`). The
portable archive and the AppImage expect them to be installed. Any other
library the host links against is copied to `lib/`.

An application inside a checkout of this repository (the examples) has no
`vendor` of its own. Its bundle mirrors the checkout below `app/`: the
`vendor` directory of the checkout, the PHP package (`packages/desktop`) and
the application at its relative path.

When the manifest lacks an `applicationId` or a `version`, or the build was
given a script, the bundled `pam-desktop.json` is completed with the values
the build resolved, so the installed application runs with the identity its
packages carry. A complete manifest ships byte for byte.

## Installers

Linux portable bundles include `install.sh` and `uninstall.sh`: an atomic
per-user installation below `${XDG_DATA_HOME:-~/.local/share}/pam-desktop/apps`,
a link in `${XDG_BIN_HOME:-~/.local/bin}`, the desktop entry and the hicolor
icon. Debian packages install the bundle below `/opt/<applicationId>`, link the
launcher in `/usr/bin` and install the desktop entry and the icon; RPM packages
use the same file system layout. AppImage wraps the bundle in an AppDir with
`AppRun`, the desktop entry and the icon. macOS builds an `.app` (the bundle
below `Contents/Resources/runtime`) before the DMG, the PKG and the portable
`.zip`. Windows wraps the bundle in an MSIX or in a per-user WiX MSI whose
upgrade code is derived from `applicationId`.

Only the per-user installations update themselves: the portable bundle on
every system and the macOS `.app`. System packages, AppImages and MSIX
packages are replaced by their package manager.

## Signing

`--sign` is available on macOS and Windows; the credentials stay in the
environment, outside the project.

| Variable | Use |
| --- | --- |
| `PAM_MACOS_SIGNING_IDENTITY` | Developer ID identity for `codesign` (hardened runtime, timestamp) of the bundled binaries, the `.app` and the DMG |
| `PAM_MACOS_INSTALLER_IDENTITY` | installer identity for `pkgbuild --sign` |
| `PAM_MACOS_NOTARY_PROFILE` | optional `notarytool` keychain profile: the DMG and the PKG are notarized and stapled |
| `PAM_WINDOWS_CERTIFICATE_SHA1` | thumbprint of the Authenticode certificate for `signtool` (SHA-256, RFC 3161 timestamp) of every `.exe` and `.dll`, the MSIX and the MSI |
| `PAM_WINDOWS_TIMESTAMP_URL` | timestamp server, DigiCert by default |
| `PAM_WINDOWS_PUBLISHER` | `Publisher` of the MSIX identity, which must match the certificate |

Missing credentials stop the build before it starts. Linux artifacts are not
signed by the build: their integrity is the per-file manifest and the
separately signed update feed described in [Signed updates](updates.md).

## Integrity and reproducibility

Every bundle contains `manifest.json` (schema 2) with:

- the identifier, name, version, build, description, publisher, category and
  update settings of the application;
- the project directory, the entry script and the launcher, relative to the
  bundle;
- the versions of PAM Desktop and of the bundled PHP;
- the operating system, architecture and ABI of the build;
- every shipped path with its byte count and SHA-256.

The updater verifies the signed hash of the archive first and every extracted
file before and after swapping bundles. Bundles of PAM Desktop 1.x (schema 1)
are not accepted.

File modes are normalized (directories, launchers, binaries and `.sh` files
0755, everything else 0644). Linux portable archives also fix the order of
entries, ownership, timestamps (`SOURCE_DATE_EPOCH`, zero by default) and the
gzip header: the same inputs give the same bytes. Build Linux artifacts on the
oldest distribution you support, since the bundled PHP libraries and the host
depend on its C library, and run an installation, launch and update test
there.

## Building the host

The host binary of a release is fetched by the Composer launcher
(`vendor/bin/pam-desktop`), which downloads
`pam-desktop-<version>-<target>.tar.gz` from the GitHub release, verifies its
published SHA-256 and keeps it in the cache of the user. `PAM_DESKTOP_HOST`
points the launcher at a local binary instead, with no download.

Building from this repository, the optional parts of the Linux backend are
Cargo features of `pam-desktop-host`, all enabled by default: `webview`
(embedded web pages through WebKitGTK 6), `tray` (the tray icon) and `hotkeys`
(global shortcuts), next to `sqlite`. A leaner host is built with, for
example, `cargo build -p pam-desktop-host --no-default-features --features
tray,sqlite`; its packages then no longer depend on WebKitGTK.
