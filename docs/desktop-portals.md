# Linux desktop portals

Opt in to user-mediated desktop integration:

```php
$app->capabilities(Capabilities::none()->desktopPortal());
```

```js
await window.pam.portal.open("https://example.com/help");
const screenshot = await window.pam.portal.screenshot();
await window.pam.portal.printPdf(
  { root: "reports", path: "annual.pdf" },
  { title: "Annual report" },
);
const camera = await window.pam.portal.camera.status();
if (camera.present) {
  await window.pam.portal.camera.request();
}
await window.pam.portal.microphone.request();
const { scanners } = await window.pam.portal.scanner.list();
await window.pam.portal.scanner.scan(
  { root: "documents", path: "receipt.png" },
  { device: scanners[0].device, resolution: 300, format: 1 },
);
```

Portal operation is an integer enum: `1` open URI, `2` interactive screenshot,
`3` print PDF, `4` query camera and `5` request camera.
Pam talks directly to xdg-desktop-portal over D-Bus, so the
desktop environment owns consent and chooser UI under Wayland, X11 and sandboxed
packages. URI opening is limited to credential-free `https`, `mailto` and `tel`.
Screenshots return an opaque read-only file grant rather than an ambient path.
Printing accepts only a capability-scoped `.pdf` descriptor and uses the native
prepare/print flow.

Camera and microphone access always pass through operating-system consent.
Linux camera consent uses XDG Camera; microphone consent uses the renderer's
native `getUserMedia` surface on every supported platform and immediately stops
the probe tracks. PAM returns only availability/grant state; raw PipeWire or
media descriptors never cross the JSON bridge. After consent, rendering uses the browser media surface or a
capability-scoped Rust plugin for professional processing. Long-lived screen
capture, biometrics and arbitrary hardware sessions remain isolated plugin
territory because their native handles must not be serialized into application
JavaScript.

Linux scanner integration uses the system SANE backend through the trusted
absolute `scanimage` executable. Operation `6` lists bounded scanner metadata;
operation `7` captures PNG (`1`), JPEG (`2`) or PNM (`3`) at 75–1,200 DPI.
Device identifiers reject option injection and control bytes. Scan output is
written to a private random staging file, monitored against a 256 MiB ceiling,
and copied into the capability-scoped destination only after the scanner exits
successfully. The staging file is removed on every exit path. Missing
`sane-utils` produces an actionable capability error rather than a shell call.
