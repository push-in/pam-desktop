# Repository layout

Moved out of the README; nothing was removed.

```text
crates/
  pam-desktop-wire     frame, patch and mutation batch encoding; property and event tables
  pam-desktop-engine   layout and tree diff
  pam-desktop-ui       platform-neutral render core and the Backend trait
  pam-desktop-gtk      Linux backend (GTK 4)
  pam-desktop-appkit   macOS backend (AppKit)
  pam-desktop-win32    Windows backend (Win32, Direct2D, DirectWrite)
  pam-desktop-host     the pam-desktop binary; SQLite, secure storage, packaging and updates
packages/desktop/      the PHP package (Pam\Desktop), the Composer launcher, the worker bootstrap, the init skeleton and tests
packages/desktop-ui/   ready controls (Pam\DesktopUi)
packages/charts/       charts (Pam\Desktop\Charts), drawn with packages/canvas/
packages/desktop-printing/  native printing of text documents and receipts
examples/              one runnable application per feature
scripts/               shot.sh captures an example on a virtual display; visual.sh compares the examples with a baseline; package-host.sh packs a release archive
.github/workflows/     ci.yml (checks) and release.yml (host archives of a tag)
docs/                  this documentation
```
