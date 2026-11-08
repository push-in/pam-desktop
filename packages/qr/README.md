# PAM Desktop QR

Local QR encoding using BaconQrCode and native PAM Canvas on Linux, macOS and Windows. No remote image service or raster extension is used.

Register before `App::run()`:

```php
Pam\Desktop\Qr\Qr::register();
```

```html
<QrCode :data="$copyPaste" size="256" />
```

`correction` accepts the sequential `ErrorCorrection` enum (`Low=1`, `Medium=2`, `Quartile=3`, `High=4`). The default is Medium. A four-module quiet zone is always preserved, modules have integral sizes, and an undersized output is rejected instead of producing unreadable geometry.

The immutable `QrCode::make($data, 256)` API exposes `scene`, `moduleCount` and `moduleSize`. Scenes batch horizontal dark runs into bounded Canvas paths, including maximum QR version 40. Encoding happens once per object. Declarative tags keep at most four scenes; `Qr::clearCache()` releases them on logout. No timers or background tasks run.

The encoder may reject content beyond the selected QR capacity. These errors surface through PAM's normal component error boundary.
