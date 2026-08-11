# Linux secrets

Pam Desktop integrates with the freedesktop Secret Service instead of writing
credentials into application files. Enable it explicitly:

```php
$app->capabilities(Capabilities::none()->secrets());
```

```js
await window.pam.secrets.set("api-token", token);
const token = await window.pam.secrets.get("api-token");
await window.pam.secrets.delete("api-token");
```

Operations are integer contracts internally (`1` read, `2` write, `3` delete).
Keys use Pam identifiers, values are UTF-8 and limited to 64 KiB. Items are
scoped by reverse-DNS application ID and key, replacement is atomic from the
Secret Service API, and the D-Bus session negotiates encrypted Diffie-Hellman
transport. A locked collection may display the desktop environment's native
unlock prompt. Missing keys return `null`; secrets never enter diagnostics or
logs.

Headless Linux sessions without a Secret Service receive a typed native error.
Pam deliberately does not fall back to plaintext files.
