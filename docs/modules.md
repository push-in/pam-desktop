# Modules

Modules are the services the host offers to PHP: storage, files, the network, databases, timers and a few system dialogs. This page describes each one and what it does when a call fails.

`examples/modules` exercises timers, storage, alerts, the clipboard, device information and the appearance preference.

## How a module call works

Every call is asynchronous. PHP sends a request to the host and returns at once; the result arrives later as a callback, and the component renders again after the callback returns.

```php
Storage::get('launches', function (?string $value): void {
    $this->launches = (int) $value + 1;
});
```

- A request gets exactly one answer: success with a payload, or failure with a message.
- Blocking work (disk, network, SQLite) runs on host threads, never on the UI thread and never in PHP.
- After a hot reload, answers to requests made by the previous worker are dropped.

### Failures

Each API reports a failure in one of three ways. The sections below say which.

| Style | What happens on failure |
| --- | --- |
| Failure closure | An optional `$failure` (or `$onError`) argument receives the message or the exception. Without it, the failure is thrown. |
| Thrown | A `RuntimeException` or a `NativeModuleException` is thrown from the callback. |
| Quiet | The success callback receives `null`, `false` or an empty value. |

An exception thrown from a module callback is reported like any uncaught error: in development the [error overlay](development.md#the-error-overlay) shows it as a module callback error.

## Storage

`Pam\Desktop\Storage\Storage` is a small durable key/value store for strings: preferences, the last opened document, a cached response.

```php
use Pam\Desktop\Storage\Storage;

Storage::set('theme', 'dark');

Storage::get('theme', function (?string $value): void {
    $this->theme = $value ?? 'light';
});
```

- Values live in one JSON file, `storage.json`, in the data directory. Writes are committed atomically, and a `get` after a `set` sees the new value.
- A value holds at most 262,144 bytes. `StorageBudget::encodeList()` and `encodeObject()` trim JSON to fit.
- Keys match `[A-Za-z0-9_.-]{1,128}`. Any other identifier is mapped to one by `Storage::safeKey()`, which `get` and `set` apply for you.

| Call | Success | Failure |
| --- | --- | --- |
| `get($key, $callback, $onError)` | `?string`; `null` when the key has no value | `$onError(NativeModuleException)`, or thrown |
| `set($key, $value, $callback, $onError)` | `$callback()` | `$onError(NativeModuleException)`, or thrown |

## Secure storage

`Pam\Desktop\Storage\SecureStorage` keeps small secrets (tokens, PIN hashes, keys) in the keychain of the system: the Secret Service on Linux, the Keychain on macOS, the Credential Manager on Windows.

```php
use Pam\Desktop\Storage\SecureStorage;

SecureStorage::set('api-token', $token, function (bool $stored): void {
    $this->signedIn = $stored;
});

SecureStorage::get('api-token', function (?string $token): void {
    $this->token = $token;
});
```

- A key holds 1 to 256 characters and a value at most 65,536 characters; longer arguments throw `InvalidArgumentException` before the call.
- Each secret is one keychain entry of the application. A value too large for a keychain entry is stored in a file under `secure/` in the data directory, encrypted with AES-256-GCM under a random key that is itself the keychain entry.
- Nothing is ever stored in the clear. When no keychain can be reached (a Linux session without a Secret Service, for instance), every call fails.

| Call | Success | Failure |
| --- | --- | --- |
| `get($key, $callback)` | `?string`; `null` when the key has no value | Quiet: `null` |
| `set($key, $value, $callback)` | `true` | Quiet: `false` |
| `delete($key, $callback)` | `true` | Quiet: `false` |

Because `get` reports `null` both for "no value" and for "keychain unavailable", treat `null` as "ask the user again", not as proof that nothing was stored.

## Files

`Pam\Desktop\System\Files` is the application's private file sandbox. Every path is relative to `pam-files/` in the data directory. A path that would leave that directory, including through a symbolic link, is refused.

```php
use Pam\Desktop\FileReference;
use Pam\Desktop\System\Files;

Files::write('notes/today.txt', $this->text, function (): void {
    $this->saved = true;
});

Files::read('notes/today.txt', function (string $contents): void {
    $this->text = $contents;
});

Files::list('notes', function (array $files): void {
    $this->names = array_map(static fn (FileReference $file): string => $file->name, $files);
});
```

File contents cross the bridge only in `read` and `write`, one mebibyte at most. Everything else (copies, downloads, imports) moves bytes inside the host and hands PHP a `FileReference` with `path`, `name`, `mimeType` and `size`. `FileReference::uri()` returns a `pam-file:///` URI that `Image` and `MediaPlayer` accept as a source.

| Call | Success | Failure |
| --- | --- | --- |
| `read($path, $callback)` | `string` | Thrown (`RuntimeException`) |
| `write($path, $contents, $callback)` | `$callback()` | Thrown |
| `list($directory, $callback)` | `list<FileReference>` | Thrown |
| `delete($path, $callback)` | `$callback()` | Thrown |
| `exists($path, $callback, $failure)` | `bool`; directories report `false` | Failure closure. Only an invalid path fails. |
| `find($path, $callback, $failure)` | `?FileReference`; `null` when the file is missing | Failure closure |
| `stat($path, $callback, $missing, $failure)` | `FileReference` | A missing file calls `$missing`; other failures go to the failure closure |
| `move($from, $to, $callback, $failure, $overwrite)` | `FileReference` | Failure closure |
| `copy($from, $to, $callback, $failure, $overwrite)` | `FileReference` | Failure closure |
| `makeDirectory($path, $callback, $failure)` | The normalized relative path | Failure closure |
| `sha256($path, $callback, $failure)` | The hexadecimal digest | Failure closure |
| `copyAsset($assetPath, $destination, $callback)` | `FileReference` of the copy of a bundled file | Thrown |
| `open($path, $mimeType, $callback, $failure)` | `$callback()` once the file opened in its default application | Failure closure |

Typical failure messages are "File does not exist", "Destination already exists" and "Source does not exist".

### Downloads

```php
use Pam\Desktop\FileDownloadProgress;

Files::downloadWithProgress(
    'https://example.com/report.pdf',
    'downloads/report.pdf',
    function (FileReference $file): void {
        $this->report = $file->path;
    },
    function (FileDownloadProgress $progress): void {
        $this->received = $progress->bytesWritten;
    },
    failure: function (string $message): void {
        $this->error = $message;
    },
);
```

`download()` and `downloadWithProgress()` fetch an HTTPS resource straight into the sandbox. The URL must be absolute, HTTPS and without credentials; the limit is 64 MiB by default and 256 MiB at most. Invalid arguments throw `InvalidArgumentException` before the call. `downloadWithProgress()` returns an id for `Files::cancelDownload()`.

### Pickers

`Files::pick()` and `Files::pickMany()` open the native file dialog and copy what the user chose into the sandbox, off the UI thread. PHP receives references to the copies.

```php
use Pam\Desktop\MediaPickerType;

Files::pick(MediaPickerType::Image, function (?FileReference $file): void {
    if ($file !== null) {
        $this->avatar = $file->uri();
    }
}, failure: function (string $message): void {
    $this->error = $message;
});
```

- `MediaPickerType` is `Image`, `Video`, `Audio`, `Media` or `Any`.
- A cancelled dialog is not a failure: `pick` reports `null` and `pickMany` an empty list.
- A failed import goes to the failure closure, or is thrown without one.

To get the real path of a user's file instead of a private copy, use `Dialog`: see [Desktop shell](desktop-shell.md#file-dialogs).

### Not available on desktop

`Share` and the file-sharing calls fail with "Sharing is not available on this platform". `Files::importUri()` fails with "Only content URIs can be imported".

## HTTP

`Pam\Desktop\Http\Http` makes network requests from host threads.

```php
use Pam\Desktop\Http\Http;
use Pam\Desktop\Http\HttpResponse;

Http::get('https://api.example.com/invoices', function (HttpResponse $response): void {
    if ($response->transportFailed()) {
        $this->error = $response->error;
        return;
    }
    if ($response->successful()) {
        $this->invoices = json_decode($response->body, true);
    }
});

Http::json('POST', 'https://api.example.com/invoices', ['total' => 4200], function (HttpResponse $response): void {
    $this->created = $response->statusCode === 201;
});
```

Rules enforced by the host:

- Requests go to `https://` only. Plain `http://` is accepted for loopback hosts, and for any host in `pam-desktop dev`.
- Redirects are never followed: a 3xx response is returned as it is.
- Certificates are verified against the trust store of the system.
- A request body holds at most one mebibyte and a response body at most 900 KiB; a larger response fails. The methods are GET, POST, PUT, PATCH and DELETE. The timeout is 30 seconds by default, from 1 to 120 seconds.

| Call | Notes |
| --- | --- |
| `Http::get($url, $callback)` | |
| `Http::post`, `put`, `patch`, `delete($url, $callback, $headers, $body, $timeoutMs)` | |
| `Http::request($method, $url, $callback, $headers, $body, $timeoutMs)` | |
| `Http::json($method, $url, $data, $callback, $headers, $timeoutMs)` | Encodes `$data` and sets the JSON headers. |
| `Http::upload(...)`, `Http::uploadWithProgress(...)` | PUT a file of the sandbox without reading it into PHP. |
| `Http::multipart($url)` | Starts a multipart form upload. |

**Failure: quiet.** The callback always receives an `HttpResponse`. When no response arrived (offline, DNS, TLS, timeout, refused URL) its `statusCode` is `0`, its `error` holds the message and `transportFailed()` is `true`. An HTTP error status is a normal response: check `successful()` or `statusCode`.

`Http::create($baseUrl, $headers, $timeout, $retry)` returns an `HttpClient` with a base URL, default headers and interceptors. Its requests return a pending response with `then()` and `catch()` instead of taking a callback; a transport failure or a status of 400 and above rejects with an `HttpError`.

## SQLite

`Pam\Desktop\Database\SQLite` gives the application local databases. A database is a file under `pam-databases/` in the data directory, opened on first use. Every call runs on one host thread, so results arrive in call order.

```php
use Pam\Desktop\Database\SQLite;
use Pam\Desktop\Modules\NativeModuleException;

SQLite::execute('app.db', 'CREATE TABLE IF NOT EXISTS notes (id INTEGER PRIMARY KEY, body TEXT NOT NULL)');

SQLite::execute('app.db', 'INSERT INTO notes (body) VALUES (?)', [$this->draft]);

SQLite::query(
    'app.db',
    'SELECT id, body FROM notes ORDER BY id DESC LIMIT ?',
    [50],
    function (array $rows): void {
        $this->notes = $rows;
    },
    function (NativeModuleException $error): void {
        $this->error = $error->getMessage();
    },
);
```

| Call | Meaning |
| --- | --- |
| `execute($database, $sql, $arguments, $callback, $onError)` | One statement. |
| `query($database, $sql, $arguments, $callback, $onError)` | Rows as a list of associative arrays. |
| `executeMany($database, $sql, $argumentSets, $callback, $onError)` | One prepared statement for every argument set, in one transaction. |
| `transaction($database, $statements, $callback, $onError)` | Several statements atomically. |

- A database name matches `[A-Za-z0-9_.-]{1,128}`. Arguments are scalars or `null` and are bound with their types.
- A query returns at most 1000 rows, 256 columns and about one mebibyte. Beyond that the call fails with a message that tells you to paginate. Use `LIMIT` and `OFFSET`.
- A batch holds 1 to 10,000 statements or argument sets.

**Failure: failure closure.** `$onError` receives a `NativeModuleException` whose message is the SQLite error. Without `$onError` the exception is thrown. Invalid arguments throw `InvalidArgumentException` before the call.

The module is the `sqlite` cargo feature of the host, on by default.

## Timers

`Pam\Desktop\System\Timers` schedules callbacks. The host keeps the deadlines; PHP sleeps until one fires.

```php
use Pam\Desktop\System\TimerHandle;
use Pam\Desktop\System\Timers;

$tick = Timers::every(1000, function (TimerHandle $timer): void {
    $this->seconds++;
});

Timers::timeout(3000, function (): void {
    $this->toastVisible = false;
});

$tick->cancel();
```

| Call | Meaning |
| --- | --- |
| `Timers::timeout($milliseconds, $callback, $failure)` | Runs once. Returns a `TimerHandle`. |
| `Timers::every($milliseconds, $callback, $failure)` | Runs repeatedly; the interval is at least 16 ms. Returns a `TimerHandle`. |
| `Timers::after($milliseconds, $callback, $failure)` | Runs once and cannot be cancelled. |
| `TimerHandle::cancel()`, `Timers::cancel($handle)` | Stops a timer. |

A delay is between 0 and 86,400,000 ms. The host has no interval of its own: after each tick PHP arms the next one, so a repeating timer drifts by the time PHP takes to handle a tick. Do not use a timer to animate; use an [animation](animation-and-gestures.md).

**Failure: failure closure.** `$failure` receives the message; without it a `RuntimeException` is thrown.

## Device information

```php
use Pam\Desktop\System\DeviceInfo;

DeviceInfo::get(function (DeviceInfo $info): void {
    $this->device = sprintf('%s · %s', $info->locale, $info->timeZone);
});
```

| Property | On desktop |
| --- | --- |
| `width`, `height`, `density` | The window's content size and the display scale. |
| `appearance`, `appState` | Light or dark; `AppState::Active` (the main window has the focus), `Inactive` (visible, without the focus) or `Background` (minimized or hidden). |
| `timeZone` | IANA id from `TZ`, `/etc/localtime` or `/etc/timezone`; `UTC` when unknown. |
| `locale` | BCP 47 tag from `LC_ALL`, `LC_NUMERIC` or `LANG`; `en-US` when unknown. |
| `appVersion`, `buildNumber` | `version` and `build` of `pam-desktop.json`. |
| `safeArea*`, `memoryClassMb`, `lowRamDevice`, `powerSaveMode`, `manufacturer`, `model` | Always zero, false or empty. |

**Failure: failure closure.** `DeviceInfo::get($callback, $failure)`.

`Platform::os()` tells the operating system synchronously; see [Desktop shell](desktop-shell.md#platform-checks).

## Device status

`Pam\Desktop\System\DeviceStatus` reports the battery, the kind of network and the power saving mode, once or on every change.

```php
use Pam\Desktop\DeviceStatus as Status;
use Pam\Desktop\NetworkType;
use Pam\Desktop\System\DeviceStatus;

DeviceStatus::read(function (Status $status): void {
    $this->battery = $status->batteryLevel;
});

$watch = DeviceStatus::watch(function (Status $status): void {
    $this->offline = $status->networkType === NetworkType::None;
});

DeviceStatus::unwatch($watch);
```

| Property | Meaning |
| --- | --- |
| `batteryLevel` | Charge from 0 to 1; `-1` when the machine has no battery or the level is unknown. |
| `charging` | The battery is charging, or full. |
| `networkType` | `NetworkType::None`, `Wifi`, `Cellular`, `Ethernet` or `Other`. |
| `expensiveNetwork` | `true` on a cellular network. |
| `lowPowerMode` | The system is saving energy. |

`watch($callback, $intervalMs)` calls back with the current status and then only when something changed. The host samples every `$intervalMs`, 1000 by default, from 250 to 60,000; a watch costs one polling thread, which exits with the last `unwatch()`.

Where the values come from:

| | Linux | macOS | Windows |
| --- | --- | --- | --- |
| Battery | The system battery under `/sys/class/power_supply`; batteries of peripherals are skipped. | `pmset -g batt` | `GetSystemPowerStatus` |
| Network | `/sys/class/net`: a wired interface that is up, else a wireless one, else none. Never `Cellular`. | The interface of the default route (`route`), Wi-Fi or Ethernet according to `networksetup`. Never `Cellular`. | `GetAdaptersAddresses`: the connected adapters that have a gateway. |
| Power saving | The ACPI platform profile is `low-power` or `quiet`. | Low Power Mode in `pmset -g`. | Battery saver is on. |

On macOS the system tools are asked at most every 10 seconds. The macOS and Windows columns are written and type-checked but have not run on a real machine.

**Failure: thrown.** A failed read throws a `RuntimeException` from the callback.

## Linking

`Pam\Desktop\System\Linking` opens URLs in the default application of the system and reports the URL the application was launched with.

```php
use Pam\Desktop\System\Linking;

Linking::open('https://example.com/help', failure: function (string $message): void {
    $this->error = $message;
});

Linking::initial(function (?string $url): void {
    if ($url !== null) {
        $this->route($url);
    }
});
```

| Call | Success | Failure |
| --- | --- | --- |
| `open($url, $opened, $failure)` | `$opened()` | Failure closure. "Unsupported URL scheme" for anything other than `https`, `http`, `mailto`, `tel` and `geo`. |
| `canOpen($url, $callback, $failure)` | `bool`: whether the scheme is one of those | Failure closure |
| `initial($callback, $failure)` | `?string`: the launch URL, once | Failure closure |
| `listen($callback)` | Each URL that arrives while the application runs | Thrown |

The launch URL is the `url` argument of `pam-desktop run` or `dev`, or else the value of `PAM_DESKTOP_LAUNCH_URL` when the host starts. `initial()` reports it once.

`listen()` receives each link that reaches the application while it runs and returns an id for `Linking::unsubscribe()`. Only one `listen()` may be active at a time. Links that arrive while no listener waits are kept, 32 at most, and delivered to the next one; a link longer than 8192 bytes is dropped.

### Deep links

An application is opened by links when its manifest declares `urlSchemes`; the installers register the schemes with the system. With `singleInstance` (the default under `run` for an application that declares schemes) the application runs once, and a later launch hands its link to the running instance. Both keys are described in [Distribution](distribution.md#links-and-single-instance).

How a link reaches an application that already runs:

| System | Mechanism |
| --- | --- |
| Linux | With `singleInstance` the GTK application is unique: the second launch forwards its link to the first, which presents its window, and exits. |
| macOS | The system calls `application:openURLs:` of the running application. The link that launched the application arrives the same way, so `listen()` receives it and `initial()` may be empty. |
| Windows | With `singleInstance` a named mutex tells the second launch that the application runs; it hands its link over with `WM_COPYDATA` and exits. |

A second launch without a link brings the running application to the front on Linux.

`examples/links` claims the `pamlinks` scheme and shows the launch link and every link received later:

```bash
target/debug/pam-desktop run examples/links pamlinks://first
target/debug/pam-desktop run examples/links pamlinks://second
```

The second command hands `pamlinks://second` to the window of the first and returns. The Linux mechanism runs; the macOS and Windows ones are written and type-checked but have not run on a real machine.

## Screen capture protection

`Pam\Desktop\System\Screen` asks the system to keep every window of the application, current and future, out of screenshots and screen recordings.

```php
use Pam\Desktop\System\Screen;

Screen::secure(true, function (bool $enforced): void {
    $this->protected = $enforced;
});

Screen::claim('app-lock');
Screen::release('app-lock');
```

| Call | Meaning |
| --- | --- |
| `Screen::secure($enabled, $callback)` | Turns the protection on or off for the whole application. `$callback` receives whether the system enforces it. |
| `Screen::claim($owner, $callback)`, `Screen::release($owner, $callback)` | A named claim, for independent features. The protection stays on while any claim is held. |
| `Screen::claimed($owner)` | Whether that claim is held. |
| `Screen::isSecure()` | Whether the application asks for protection, by `secure()`, a claim or a route. It does not tell whether the system enforces it. |

A route asks for protection while it is the active route of its navigator: `new ScreenOptions(secure: true)`, or `->secure()` on a route declared with `Route::screen()`.

| System | What happens |
| --- | --- |
| Linux | Not enforced. The GTK backend does not implement it and the callback receives `false`. |
| macOS | The sharing type of every window is set to none. Not run on a real machine. |
| Windows | Every window is excluded from captures; on a system that cannot exclude a window, it is captured as a black rectangle. Not run on a real machine. |

## Toasts

`Pam\Desktop\System\Toast` shows a short message over the application. The host draws it; one toast shows at a time and a new one replaces it. Pressing a toast dismisses it, and presses outside it reach the application.

```php
use Pam\Desktop\System\Toast;
use Pam\Desktop\System\ToastPosition;

Toast::show('Copied to the clipboard');
Toast::success('Saved', 'Your changes are safe.');
Toast::error('No connection', 'Trying again in 5 seconds.', ToastPosition::Bottom);
Toast::message('3 new messages', fitContent: true);
```

| Call | Result |
| --- | --- |
| `Toast::show($message, $long)` | A plain capsule near the bottom of the window. |
| `Toast::success`, `error`, `info($title, $message, $position)` | A card with a title, a message and an accent strip, at the top or the bottom. |
| `Toast::message($title, $message, $type, $position, $durationMs, …)` | The card with every option: duration (500 to 60,000 ms), offset, colors, sizes, corner radius, `fitContent`, an icon. |

**Failure:** an empty message or title throws `InvalidArgumentException` before the call. Toasts report nothing back.

## Alerts

`Pam\Desktop\System\Alert` shows a native message dialog.

```php
use Pam\Desktop\System\Alert;
use Pam\Desktop\System\AlertButton;

Alert::choose(
    'Save changes?',
    'Your edits will be lost if you do not save them.',
    [AlertButton::cancel('Cancel'), AlertButton::destructive('Discard'), AlertButton::make('Save')],
    function (int $index): void {
        $this->answer = ['Cancelled', 'Discarded', 'Saved'][$index] ?? 'Dismissed';
    },
);
```

| Call | Success | Failure |
| --- | --- | --- |
| `Alert::show($title, $message, $dismissed)` | `$dismissed()` | Thrown |
| `Alert::choose($title, $message, $buttons, $pressed, $failure)` | The index of the pressed button, or `Alert::DISMISSED` (`-1`) when the dialog closed another way | Failure closure |

`choose()` takes one to three buttons, at most one of them `AlertButton::cancel()`; anything else throws `InvalidArgumentException` before the call.

## Clipboard

`Pam\Desktop\System\Clipboard` is described in [Desktop shell](desktop-shell.md#clipboard). **Failure: quiet.** `setText` reports `false`, `getText` reports `null`, `hasText` reports `false`.

## Updates

Application updates are a host module as well. They have their own page: [Updates](updates.md).

## Capabilities that are not part of PAM Desktop

PAM Desktop shares its component syntax with PAM Native, the mobile sibling. Capabilities that only exist on phones are not included: location, contacts, SMS, sensors, biometrics, push notifications, the camera and media library, audio recording.

Their class names are still known, so that shared code fails with a clear message. Using one, for example `Pam\Desktop\System\Location`, throws a `Pam\Desktop\Plugin\PluginException`:

```text
Pam\Desktop\System\Location is not part of PAM Desktop: it belongs to the optional package
pushinbr/pam-desktop-location (Pam\Desktop\Location\Location), which is not installed.
```

A few calls of the shared API exist but have nothing to do on a desktop and answer with a fixed value: permission checks report "granted", and haptics and vibration succeed without effect.
