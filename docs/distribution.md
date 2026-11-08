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
    "extensions": ["auto", "intl"],
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
| `extensions` | the shared PHP extensions the application refers to | lowercase PHP extension names to ship instead; `"auto"` in the list adds the ones the application refers to, `"all"` ships every one PHP loads on the build machine |
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
`dist`, `node_modules`, `target`, any directory tagged as a cache with a
`CACHEDIR.TAG` (Cargo targets under any name, for instance), the output
directory itself and the nested
`vendor` directories of vendored packages. `type=path` repositories and the
local paths Composer recorded for them are removed from the bundled
`composer.json` and `composer.lock`.

A bundle holds no symbolic links. A link is replaced by a copy of what it
points at. A link that leaves the project is only accepted below the
project's own `vendor/` folder and only when it resolves into a package that
`vendor/composer/installed.json` lists there (how Composer links `path`
repositories); a folder that is merely named `vendor` elsewhere, a package
Composer did not install, and a link inside a linked package that leaves that
package stop the build, as do a link into the output directory and a cycle of
links.

At run time the component compiler follows the same idea: a symlink inside a
component folder may lead into the project (`PAM_DESKTOP_PROJECT_ROOT`, else
the nearest folder above the components with a `composer.json`), its `vendor/`
folder, a package installed there, or a `path` repository declared in the
project's `composer.json`; anything else is refused with an error naming the
link. Fonts named `asset://...` are resolved to their real path and must stay
inside the application.

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
`PATH` or in `~/.local/bin` (started as `pam exec`), then `php` on `PATH`, and
on Linux and macOS the PAM runtime is installed when none exists, after
consent (a prompt in a terminal, or `--yes` / `PAM_DESKTOP_AUTO_INSTALL=1`
for CI): its archive must match the size and SHA-256 of the PAM release
manifest, whose Ed25519 signature is checked against the pinned PAM release
key, and it is unpacked in Rust (see
[How the host finds PHP](getting-started.md#how-the-host-finds-php-and-the-package)).
PAM has no Windows release, so on Windows the order is `PAM_DESKTOP_PHP`, the
PHP that PAM Desktop installed, `php.exe` on `PATH`, and, when there is none
and the user agrees, the newest official PHP 8.5 build of windows.php.net (non
thread safe, x64) is downloaded into `%LOCALAPPDATA%\PAM Desktop\php\<version>`:
its archive must match both the SHA-256 that windows.php.net publishes in
`releases.json` and the one PAM Desktop pins for that build, before anything
is extracted or run (with the `tar.exe` of `System32`, or its PowerShell
before 1803); a build newer than the pinned ones is replaced by the newest
pinned build unless `PAM_DESKTOP_PHP_ALLOW_UNPINNED=1`. Its `php.ini` turns on the common shared
extensions the archive has (`curl`, `exif`, `fileinfo`, `gd`, `intl`,
`mbstring`, `openssl`, `pdo_sqlite`, `sodium`, `sqlite3`, `zip`); the other
extensions an application lists in `extensions` are turned on for it by
`run` and `dev`. PHP's curl and OpenSSL have no certificate store of their
own on Windows: the host writes a CA bundle of the Windows store's roots and
Mozilla's (`webpki-root-certs`, updated with PAM Desktop) to the cache
directory of the application at every start, rewriting it only when it
changed, and passes it to that PHP and to the PHP of a bundle as
`curl.cainfo` and `openssl.cafile`, so `curl` and `https://` streams verify
servers. The official build needs the Visual C++ Redistributable
2015-2022 (x64), recent enough for the build (PHP refuses an older
`vcruntime140.dll`); `run` and `build` say so when it is missing. The build asks it for its
version, its extension directory and the extensions it loads. An extension
compiled into the binary needs nothing; a shared one is a file to ship, with
the libraries it links against, and those add up: on Ubuntu, every extension
of a common PHP installation is about 90 MB, of which `intl` alone is 39 MB.

So the build ships the shared extensions the application refers to. It runs
PHP over the `.php` and `.pam` files that go into the bundle, the packages in
`vendor` included (those Composer installed for development only are skipped),
and keeps an extension when a file names one of its functions, classes or
constants, when a string holds one of its functions as a callable, a PDO data
source (`sqlite:`) or a stream wrapper it provides (`phar://`), or when a
`composer.json` requires it (`ext-intl`). Extensions another kept extension
cannot load without are kept too, and so are `openssl` and `zlib`. A name that
only appears in a guard, such as `extension_loaded('intl')`, does not count:
that code works without the extension. The build prints what it shipped and
what it left out; `PAM_DESKTOP_EXTENSION_USAGE=1` also prints the first
reference found to each extension.

What the search cannot see is a name put together at run time
(`$function = 'mb_'.$name`). For that, list the extension: `"extensions":
["auto", "intl"]` ships `intl` and whatever is referred to. A list without
`"auto"` ships exactly the listed extensions, and `["all"]` every shared
extension PHP loads on the build machine. An extension that is listed and is
neither loaded nor present in the extension directory stops the build. Extensions are loaded from the
bundled `php.ini` only: the configuration of the build machine is not shipped,
and the configuration of the machine the application runs on is not read
(`PHPRC` points at the bundle and `PHP_INI_SCAN_DIR` is empty). On Linux the
shared libraries of the binary and of each extension are collected with `ldd`
and bundled, except the C library itself. On macOS the libraries that sit
beside the binary are bundled: give the build a relocatable PHP, such as a
static build. On Windows the build reads the import tables of `php.exe` and
of the shipped extensions and bundles only the DLLs beside PHP they load (the
official archive also carries glib, enchant, libpq and phpdbg, which most
applications never load), plus the Visual C++ runtime DLLs PHP needs, copied
from `System32` of the build machine (app-local deployment, so the machine
the application is installed on needs no redistributable). A host built
with the GNU toolchain loads `WebView2Loader.dll`, which must sit beside it;
the MSVC toolchain links it statically.

**The launcher** starts `pam-desktop run` on the bundled project with
`PAM_DESKTOP_PHP` pointing at the bundled PHP, and exports
`PAM_DESKTOP_BUNDLE_ROOT` and `PAM_DESKTOP_LAUNCHER` for the updater. On Linux
and macOS it is a shell script that resolves links to itself; PHP's own
libraries and configuration are set by `bin/pam-desktop-php`, for the PHP
process alone. On Windows the launcher is the host under the name of the
application: started from `bin` of a bundle under a name other than
`pam-desktop`, the host runs the bundle. The build writes the icon of the
application into that copy, at 16, 32, 48 and 256 pixels, before anything is
signed. The icon is made from a PNG: with an SVG icon, or none, the launcher
has no icon of its own and the build says so in a note.

**The system provides the desktop platform.** On Linux the host uses the GTK 4
of the machine it runs on, with its GStreamer, and WebKitGTK 6 for the
`WebView` element, so that themes, portals and GPU drivers match. These libraries and
everything they depend on are not bundled; `.deb` and `.rpm` packages declare
GTK as a dependency (`libgtk-4-1`; `gtk4`). The GTK host plays media through
the GStreamer module of GTK, which it does not link: that module comes with
GTK's own recommendations (`libgtk-4-media-gstreamer` and
`gstreamer1.0-plugins-base` on Ubuntu 24.04, installed by a default
`apt install`), and the GStreamer plugins are declared
(`gstreamer1.0-plugins-base`, `gstreamer1.0-plugins-good`;
`gstreamer1-plugins-base`, `gstreamer1-plugins-good`) only for a host that
links GStreamer itself. An application that plays media should tell its users
to install `gstreamer1.0-plugins-good` for the common formats. WebKitGTK (`libwebkitgtk-6.0-4`; `webkitgtk6.0`)
is declared only when a `.php` or `.pam` file of the application, outside
`vendor`, names `WebView`: the host loads it on the first web view. The
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

Building from this repository, the optional parts of the backends are Cargo
features of `pam-desktop-host`, all enabled by default: `webview` (embedded
web pages through WebView2 on Windows), `tray` (the
tray icon on Linux) and `hotkeys` (global shortcuts on Linux), next to
`sqlite`. A leaner host is built with, for
example, `cargo build -p pam-desktop-host --no-default-features --features
tray,sqlite`.

## Verified on Linux

Release host (`cargo build --release`), built and packaged on Ubuntu 24.04
(x86_64) with the PAM runtime as PHP, on 2026-11-08. The test application was
a real point-of-sale application (an `index.php`, ~2 MB of PHP and templates,
four Composer packages, a link scheme for OAuth). Everything below ran in a
throwaway Ubuntu 24.04 container without network access to anything but a
local test server.

| Format | Result |
| --- | --- |
| `directory` | 123 MiB: host 11 MiB, PHP 9.4 MiB, PHP libraries 86 MiB (ICU data alone 29 MiB, `gd` with its AVIF/HEIF codecs about 30 MiB), application 15 MiB |
| `portable` | 42 MiB `.tar.gz`; `install.sh` installs per user, registers the desktop entry, the icon and the scheme (`gio mime x-scheme-handler/<scheme>` names the entry) |
| `deb` | 32 MiB; `apt install ./<package>.deb` on a minimal Ubuntu 24.04 pulled 101 packages (33 MiB download, GTK 4 and its recommendations); `Installed-Size` 120 MiB; `desktop-file-validate` clean |
| `appimage` | 39 MiB, built with the official `appimagetool` (it needs `file` on `PATH`); runs with `APPIMAGE_EXTRACT_AND_RUN=1` where FUSE is missing |
| `rpm` | not exercised (no `rpmbuild` on the build machine) |

Checked in the bundle: no `.git`, `.env*`, `tests`, `target`, Cargo caches or
`evidence`; no symbolic links; nothing writable by others; directories,
launchers and binaries 0755, the rest 0644 (root-owned in the `.deb`). Two
things the build does not decide for you: files at the project root that only
serve development (visual fixtures, notes) ship unless `exclude` lists them,
and a Composer `path` package ships the whole directory it points at, minus
the trees listed above (`crates`, `docs`, `examples`, `tests`, nested
`vendor`), so its README, CHANGELOG, CI files and scripts come along.

The installed application never writes below `/opt`: compiled templates go to
`~/.cache/<applicationId>` and state to `~/.local/state/<applicationId>`.

Links: with the `.deb` installed, `gio open '<scheme>://…'` started the closed
application and delivered the link (cold start), and `xdg-open '<scheme>://…'`
with the application open handed the link to the running process, which
stayed the only one (`singleInstance`).

Cold start of the installed `.deb` under Xvfb (software GL), from the launcher
to the window on screen and to the first screen painted, median of 10 runs
with a warm template cache: 249 ms and 377 ms. The very first start, which
fills `~/.cache`, took 599 ms and 768 ms. Resident memory after start: host
185 MiB (with Mesa's software renderer), PHP worker 55 MiB.
