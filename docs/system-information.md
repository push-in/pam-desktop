# Linux system information

System inspection is disabled by default because hardware and connectivity
data can contribute to fingerprinting. Enable it deliberately in PHP:

```php
$app->capabilities(
    Capabilities::none()->systemInformation(),
);
```

Read one bounded snapshot from the frontend:

```js
const snapshot = await pam.system.snapshot();
```

The result contains the operating system, architecture, logical CPU count,
total and available memory, uptime, connectivity, power state and optional
battery percentage. Unavailable Linux kernel data is returned as `null`, not
guessed.

Connectivity and power values are sequential integer enums:

| Field | Values |
| --- | --- |
| `connectivityState` | `1` offline, `2` online |
| `powerState` | `1` unknown, `2` charging, `3` discharging, `4` full |

Connectivity means that a non-loopback Linux interface reports `up`; it is not
a promise that a particular internet endpoint is reachable. No hostname,
username, IP address, network name, device serial or hardware identifier is
exposed.
