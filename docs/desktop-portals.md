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
```

Portal operation is an integer enum: `1` open URI, `2` interactive screenshot,
`3` print PDF. Pam talks directly to xdg-desktop-portal over D-Bus, so the
desktop environment owns consent and chooser UI under Wayland, X11 and sandboxed
packages. URI opening is limited to credential-free `https`, `mailto` and `tel`.
Screenshots return an opaque read-only file grant rather than an ambient path.
Printing accepts only a capability-scoped `.pdf` descriptor and uses the native
prepare/print flow.

Camera, screen capture streams, biometrics and hardware devices deliberately
remain process-plugin territory: they require long-lived PipeWire/FIDO/device
sessions whose handles cannot be represented safely as JSON. The plugin SDK
keeps those sessions isolated while the browser-facing command remains narrow.
