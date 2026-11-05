# Media and web

This page covers the elements that show pictures, play media, embed pages and draw: `Image`, `MediaPlayer`, `WebView`, `Canvas` and `DrawingCanvas`. Each has a runnable example: `examples/images`, `examples/media`, `examples/webview` and `examples/canvas`.

## Image

```php
use Pam\Desktop\ImageErrorEvent;
use Pam\Desktop\ImageFit;
use Pam\Desktop\ImageLoadEvent;
use Pam\Desktop\UI\Image;

Image::make('https://picsum.photos/id/1015/600/400')
    ->fit(ImageFit::Cover)
    ->defaultSource('asset://assets/placeholder.png')
    ->onLoad(function (ImageLoadEvent $event): void {
        $this->status = sprintf('Loaded %d×%d', $event->width, $event->height);
    })
    ->onError(function (ImageErrorEvent $event): void {
        $this->status = 'Error: '.$event->message;
    })
    ->style(new Style(width: 200, height: 130, borderRadius: 14, backgroundColor: 0xFFE8ECF3));
```

Give the image a size in its style, as the examples do.

### Sources

| Source | Resolves to |
| --- | --- |
| `https://…` | Downloaded into the image cache. Plain `http://` is accepted only in `pam-desktop dev`. |
| `asset://path` | A file of the project, relative to the project directory. |
| A relative path | The same as `asset://`. |
| `pam-file:///path` | A file of the application's sandbox. `FileReference::uri()` builds this URI. |
| An absolute path or `file://` URL | That file. This is what file dialogs and file drops hand out. |
| `data:image/…` | The encoded bytes, base64 or percent-encoded, up to 8 MiB. |

Decoding is done by the platform, off the UI thread, so the formats are the ones the platform decodes. An image is decoded no larger than the box that shows it needs, so a thumbnail of a large photo stays small in memory.

### Fit

`fit()` takes an `ImageFit`:

| Value | Result |
| --- | --- |
| `Cover` | Fills the box and crops the overflow, keeping the aspect ratio. The default. |
| `Contain` | Fits inside the box, keeping the aspect ratio. |
| `Fill` | Stretches to the box. |
| `Center` | Natural size, centred. |
| `Repeat` | Natural size, tiled from the top-left corner. |

The image follows the border radius and the borders of its box.

### Placeholders and tint

| Method | Meaning |
| --- | --- |
| `defaultSource($source)` | Shown while the image loads and when it fails. |
| `loadingIndicatorSource($source)` | Shown while the image loads; wins over `defaultSource`. |
| `fadeDuration($milliseconds)` | Fade-in of a loaded image; 0 disables it. |
| `tint($color)` | Replaces every opaque pixel with one color (template images, icons). |
| `overlayColor($color)` | A color painted over the image. |

When the source changes, the previous picture stays on screen until the new one is decoded.

### Cache policy

Remote images are stored in the cache directory under `pam-images-v1/`, one file per image, named by the SHA-256 of its URL. `cache()` chooses how a remote image uses it:

| `ImageCachePolicy` | Behaviour |
| --- | --- |
| `Default`, `ForceCache` | Use the cached file when there is one; otherwise download and keep it. |
| `Reload` | Delete the cached file, download again and keep the new one. |
| `OnlyIfCached` | Never touch the network. Fails with "Image is not available in the local cache." when the file is absent. |
| `None` | Download every time and keep nothing on disk. |

`headers([...])` adds request headers to the download.

`Image::prefetch($urls)` downloads images into the same cache ahead of rendering, so the `Image` that shows them later is a cache hit.

Decoded images are shared between elements that show the same source at a similar size. A decoded image that no element shows stays in memory for a while, in a bounded least-recently-used set (48 images, 96 MiB), so scrolling a list back does not decode again.

### Events

| Method | Handler receives |
| --- | --- |
| `onLoadStart` | nothing |
| `onProgress` | `ImageProgressEvent` (`loaded`, `total` bytes) |
| `onLoad` | `ImageLoadEvent` (`uri`, `width`, `height` in image pixels) |
| `onError` | `ImageErrorEvent` (`message`) |
| `onLoadEnd` | nothing |

### `ImageBackground`

`ImageBackground::make($source, ...$children)` is a box whose background is a picture; its children are laid out above it. It takes the same methods as `Image`.

## MediaPlayer

```php
use Pam\Desktop\MediaClockFormat;
use Pam\Desktop\MediaReadyEvent;
use Pam\Desktop\UI\MediaPlayer;

MediaPlayer::make('asset://assets/clip.mp4')
    ->thumbnail('asset://assets/poster.jpg')
    ->autoPlay($this->playing)
    ->muted($this->muted)
    ->loop($this->loop)
    ->currentTime($this->seek)
    ->fit(ImageFit::Contain)
    ->nativeRef('clip')
    ->onReady(function (MediaReadyEvent $event): void {
        $this->status = sprintf('Ready · %d×%d · %.1f s', $event->naturalWidth, $event->naturalHeight, $event->duration);
    })
    ->onEnd(function (): void {
        $this->playing = false;
    })
    ->onError(function (string $message): void {
        $this->status = 'Error: '.$message;
    })
    ->style(new Style(height: 380, borderRadius: 18, backgroundColor: 0xFF12161C));
```

`MediaPlayer::make($source, MediaType::Video)` plays video; `MediaType::Audio` plays audio. The player is the platform's: GStreamer through GTK on Linux, AVFoundation on macOS, Media Foundation on Windows.

Sources are `https://` URLs, `asset://` paths, `pam-file:///` URIs, absolute paths, `file://` URLs, and relative paths, which resolve in the file sandbox.

### Play intent

`autoPlay($bool)` is the play intent, not a command: `true` means "this player should be playing", `false` means "paused". Keep it in state and flip it to play and pause.

The intent follows the property only when the property changes. If the user pauses with the native controls, an unrelated render does not start playback again.

| Method | Meaning |
| --- | --- |
| `autoPlay(bool)` | The play intent. |
| `controls(bool)` | Shows the platform's transport controls. On by default. |
| `loop(bool)`, `muted(bool)`, `volume(0..1)` | Applied live. |
| `playbackRate(0.25..4)` | Backends without rate control play at normal speed. |
| `fit(ImageFit)` | How the video fills the box. |
| `thumbnail($source)` | A poster shown until the first video frame is drawn. |

### Seeking

`currentTime($seconds)` seeks when its value changes. To seek to the same position twice, the value must differ between the two renders; `examples/media` alternates between `0.0` and `0.001` to restart the clip.

### Visibility

A player that is not visible (a covered route, an inactive tab, a hidden box) pauses at once and releases its decoder after five seconds. It is restored at the same position when it shows again. `keepAliveWhenCovered()` opts out, for audio that must continue under another screen.

`autoPlayWhenVisible($fraction, $group)` lets the host decide playback from visibility, with no round trip: the player plays while at least `$fraction` of it is on screen and pauses when it falls below that fraction minus 0.2. Players that share a `$group` play one at a time. `autoPlay` remains the intent: `false` keeps the player paused whatever its visibility.

### Cache

Remote media plays as a stream unless a cache policy asks for the disk cache (`pam-media-v1/` in the cache directory).

| `MediaCachePolicy` | Behaviour |
| --- | --- |
| `None`, `Memory` | Stream from the network. |
| `Disk`, `MemoryAndDisk`, `CacheFirst` | Play the cached file when it is fresh; otherwise download the file, then play it. |
| `NetworkFirst` | Always download. |
| `CacheOnly` | Fails with "Media is not available in the local cache." when the file is absent. |
| `StaleWhileRevalidate` | Play the cached file now and refresh it in the background. |

| Method | Meaning |
| --- | --- |
| `cache(MediaCachePolicy)` | The policy. |
| `streamingCache()` or `downloadWhilePlaying()` | Start playing from the network while the file downloads. |
| `cacheKey($key)` | A stable identity for the cached file, instead of the URL. |
| `maxAge($milliseconds)` | How long a cached file counts as fresh. |
| `maxCacheSize($bytes)` | Size budget of the media cache; the oldest files are deleted beyond it. |
| `pinOffline()` | Keeps the file when the cache is trimmed. |
| `onCacheHit`, `onCacheMiss`, `onCacheProgress`, `onCacheReady` | Receive a `MediaCacheEvent`. |

### Events

| Method | Handler receives |
| --- | --- |
| `onLoadStart` | nothing |
| `onReady` | `MediaReadyEvent` (`naturalWidth`, `naturalHeight`, `duration`) when it declares a parameter |
| `onBuffering` | `bool`: waiting for data or resumed |
| `onProgress` | `float $currentTime, float $duration`, about four times a second while playing |
| `onEnd` | nothing; not reported while looping |
| `onError` | `string $message` |

### Clock and progress bindings

A clock label and a progress bar do not need `onProgress`. Name the player with `nativeRef()` and bind other elements to it; the host updates them while the media plays and PHP does not run.

```php
Row::make(
    Text::make('0:00')
        ->mediaClock('clip', MediaClockFormat::ElapsedAndDuration)
        ->style(new Style(fontSize: 13, fontWeight: 600, width: 96)),
    Column::make(
        Column::make()->mediaProgress('clip')->style(new Style(height: 6, borderRadius: 3, backgroundColor: 0xFF2F6BFF)),
    )->style(new Style(flexGrow: 1, height: 6, borderRadius: 3, backgroundColor: 0xFFE4E7EC, overflow: Overflow::Hidden)),
)->style(new Style(gap: 12, alignItems: Align::Center));
```

- `Text::mediaClock($ref, $format)` shows the position as a clock. Formats: `Elapsed`, `Remaining`, `ElapsedAndDuration`, `Duration`, `CountdownSeconds`. The text of the element is the placeholder until the player is ready; give it a fixed width so updates do not change the layout.
- `mediaProgress($ref)` exists on every element. The element is scaled from its leading edge to `position / duration`.

## WebView

`WebView` embeds a page rendered by the web engine of the operating system: WebKitGTK on Linux, WKWebView on macOS, WebView2 on Windows. It is an element like any other; it is not how PAM Desktop renders the application.

```php
use Pam\Desktop\UI\WebView;

WebView::make('https://example.com/')
    ->allowedHosts('example.com')
    ->onLoad(function (): void {
        $this->status = 'Page loaded';
    })
    ->onError(function (string $message): void {
        $this->status = 'Error: '.$message;
    })
    ->style(new Style(flexGrow: 1, borderRadius: 16));
```

The source is an `https`, `http` or `file` URL, or inline HTML (a string that starts with `<`), up to one mebibyte. Inline HTML is loaded with an `about:blank` origin.

On Linux the host loads WebKitGTK 6 (`libwebkitgtk-6.0.so.4`) the first time a `WebView` is shown, so applications without one neither need it installed nor pay for it in memory. When the library is missing, the element reports an error through `onError`. On Windows the element needs the `webview` cargo feature of the host, which is on by default; a host built without it reports an error through `onError` for any source.

### Bridge

A page sends text to the application with `window.PamNative.postMessage(text)`. The application receives it in `onMessage`, together with the URL of the page:

```php
WebView::make(<<<'HTML'
    <!doctype html>
    <button onclick="PamNative.postMessage('clicked')">Send a message to PHP</button>
    <script>
      window.setCount = function (value) { document.title = value; };
    </script>
    HTML)
    ->injectedJavaScript("window.setCount({$this->count});")
    ->onMessage(function (string $message, string $url): void {
        $this->status = "Message: {$message}";
    });
```

The bridge exists in the main frame only. A message may hold up to 1,048,576 characters.

### Injected script

The application talks to the page through `injectedJavaScript($script)`. The script is evaluated after every page load, and at once when its text changes on a loaded page. To run the same script again, make the text differ. The sample above pushes a counter into the page each time the state changes.

### Allowed hosts

`allowedHosts(...$hosts)` limits main-frame navigation to those hosts: exact matches, no wildcards. A navigation to another host is refused and reported through `onError` as "WebView blocked navigation to <host>". Without hosts, every page is allowed. `about:` and `data:` URLs always pass.

A page that neither finishes nor fails within 30 seconds is stopped and reported through `onError`.

### Options

| Method | Meaning |
| --- | --- |
| `source($source)` | Navigates to another source. |
| `javaScriptEnabled(bool)` | On by default. |
| `domStorageEnabled(bool)` | Local and session storage. On by default. |
| `userAgent(?string)` | `null` keeps the engine's own. |
| `allowsInlineMedia(bool)` | Media may start without a click. On by default. |
| `scrollEnabled(bool)` | Whether the page scrolls. |

## Canvas

`Canvas` draws solid vector primitives from PHP. Coordinates are in the logical units of the element, from its top-left corner. Commands paint in order, each over the previous ones.

```php
use Pam\Desktop\UI\Canvas;

$chart = Canvas::make()->roundedRectangle(0, 0, 380, 200, 16, 0xFF12161C);
foreach ($this->bars as $index => $value) {
    $height = $value * 1.6;
    $x = 32 + $index * 48;
    $chart = $chart
        ->roundedRectangle($x, 180 - $height, 28, $height, 8, 0xFF2F6BFF)
        ->circle($x + 14, 180 - $height, 5, 0xFFFFFFFF);
}
$chart = $chart->line(20, 180, 360, 180, 2, 0xFF8A93A3);

$chart->style(new Style(width: 380, height: 200));
```

| Method | Draws |
| --- | --- |
| `rectangle($x, $y, $width, $height, $color)` | A filled rectangle. |
| `roundedRectangle($x, $y, $width, $height, $radius, $color)` | A filled rectangle with rounded corners. |
| `circle($centerX, $centerY, $radius, $color)` | A filled circle. |
| `line($startX, $startY, $endX, $endY, $width, $color)` | A segment with round caps. |

A canvas holds at most 10,000 commands and one mebibyte of encoded commands. The drawing is rebuilt from state on each render, like any element; the host only rasterises it.

## DrawingCanvas

`DrawingCanvas` is a freehand surface over a picture. The user draws with the pointer; the strokes are a JSON document that PHP keeps in state.

```php
use Pam\Desktop\DrawingMode;
use Pam\Desktop\UI\DrawingCanvas;

DrawingCanvas::make('asset://photo.jpg', $this->drawing)
    ->brush($this->color, 6, $this->eraser ? DrawingMode::Eraser : DrawingMode::Brush)
    ->undoRequest($this->undo)
    ->clearRequest($this->clear)
    ->onChange(function (string $drawing): void {
        $this->drawing = $drawing;
    })
    ->cursor('crosshair')
    ->style(new Style(width: 400, height: 300, borderRadius: 16, backgroundColor: 0xFF12161C));
```

| Method | Meaning |
| --- | --- |
| `make($source, $drawing)` | The picture under the strokes and the current document (`''` for none). |
| `brush($color, $width, $mode)` | Color, width (1 to 64) and `DrawingMode::Brush` or `Eraser`. |
| `undoRequest($serial)` | Removes the last stroke each time the serial changes. |
| `clearRequest($serial)` | Removes every stroke each time the serial changes. |
| `onChange` | Receives the document when it changes. |

The picture is always fitted inside the element, and stroke coordinates are stored relative to the picture, so a document drawn at one size shows correctly at another. The eraser removes strokes without touching the picture. Escape drops the stroke in progress. The document keeps the last 256 strokes.

## Related

- [Components](components.md) for the other elements.
- [Animation and gestures](animation-and-gestures.md#native-transforms) to pan and zoom an image.
- [Modules](modules.md#files) for the file sandbox behind `pam-file:///` URIs.
