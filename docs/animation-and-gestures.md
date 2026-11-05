# Animation and gestures

PHP describes motion; the host runs it. Animations, gesture tracking and the transforms that follow the pointer are sampled by the host's frame clock and never wait for PHP. PHP hears about the result: an animation completed, a gesture began or ended, a drag settled.

`examples/motion` and `examples/gestures` are the runnable versions of this page.

## Implicit transitions

`animate()` makes an element animate towards its new values whenever its opacity, translation, scale or rotation changes between two renders.

```php
use Pam\Desktop\AnimationEasing;

View::make()
    ->style(new Style(
        width: 56,
        height: 56,
        borderRadius: 16,
        backgroundColor: 0xFF2563EB,
        translationX: $this->moved ? 360.0 : 0.0,
        rotation: $this->moved ? 180.0 : 0.0,
        opacity: $this->moved ? 0.6 : 1.0,
    ))
    ->animate(600, AnimationEasing::EaseInOut);
```

`animate($durationMs = 180, $easing = AnimationEasing::EaseInOut)` takes a duration from 1 to 10000 ms. A transition starts from what is on screen at that moment, so interrupting one does not jump. An element never transitions on the frame that mounts it.

`transition()` takes a CSS transition list instead, with a duration, easing and delay for each property, and `spring(mass stiffness damping)` as a timing function:

```php
View::make()->transition('transform 300ms spring(1 260 18), opacity 120ms ease-out 40ms');
```

## Presets

`motion()` plays an entrance when the element appears.

```php
use Pam\Desktop\MotionPreset;

Text::make('Slide up')->motion(MotionPreset::SlideUp, 500);
```

The presets are `FadeIn`, `ScaleIn`, `SlideUp`, `SlideDown`, `Success` and `Shake`. `motion($preset, $durationMs = 240, $easing = AnimationEasing::EaseOut)` accepts up to 2000 ms.

## Animation programs

An `Animation` is a declarative program: a list of phases, each animating one or more properties with a timing curve or a spring. Attach it with `animation()`.

```php
use Pam\Desktop\Animation\Animation;
use Pam\Desktop\Animation\Easing;

// A spring that replays each time $this->bounces changes.
View::make()
    ->style(new Style(width: 56, height: 56, borderRadius: 28, backgroundColor: 0xFFEF4444))
    ->animation(
        Animation::sequence(
            Animation::set(['scale' => 0.4]),
            Animation::spring(['scale' => 1.0], stiffness: 180.0, damping: 8.0),
        )->key($this->bounces),
    );

// An endless rotation.
View::make()
    ->style(new Style(width: 56, height: 56, borderRadius: 12, borderWidth: 6, borderColor: 0xFF10B981))
    ->animation(Animation::timing(['rotate' => 360], 1400, Easing::Linear)->repeat());
```

| Builder | Meaning |
| --- | --- |
| `Animation::timing($to, $duration = 300, $easing = Easing::EaseInOut, $delay = 0)` | Animates every property in `$to` with a duration and an easing. |
| `Animation::spring($to, $stiffness = 100, $damping = 10, $mass = 1, $velocity = 0, $delay = 0)` | A physical spring towards `$to`. |
| `Animation::set($values)` | Jumps at once. |
| `Animation::wait($milliseconds)` | A pause. |
| `Animation::sequence(...$parts)` | Chains steps for each property. |
| `->then($next)` | Runs `$next` after every track of this animation settled. |
| `->with($other)` | Runs `$other` in parallel. |
| `->delay($milliseconds)` | Delays the first phase. |
| `->repeat($times = -1)` | Repeats; `-1` is forever. |
| `->key($value)` | The replay identity. |

Properties are `opacity`, `translateX`, `translateY`, `scale`, `scaleX`, `scaleY`, `rotate` (degrees) and `borderRadius`.

A program plays when it is attached and replays only when its identity changes: the program itself, or the value given to `key()`. Rendering the same program again does not restart it. That is why the spring above uses a counter as its key.

`Easing` has `Linear`, `Ease`, `EaseIn`, `EaseOut`, `EaseInOut`, the `Quad` and `Cubic` variants of each, `EaseOutBack`, and `Easing::bezier($x1, $y1, $x2, $y2)` for a custom curve.

## Shimmer

A loading placeholder with a moving highlight is two properties on a box:

```php
use Pam\Desktop\PropKey;

View::make()
    ->style(new Style(height: 18, borderRadius: 9, backgroundColor: 0xFFE5E7EB, widthPercent: 70))
    ->property(PropKey::ShimmerEnabled, true)
    ->property(PropKey::ShimmerGradientColor, 0xCCFFFFFF);
```

## Reduced motion

When the system asks for reduced motion, the host skips press feedback animation, animated programmatic scrolling and the settle animations of gestures: they land at their end state at once. The setting is followed while the application runs.

## Gesture detectors

`GestureDetector` wraps content and recognises one kind of gesture on it.

```php
use Pam\Desktop\GestureEvent;
use Pam\Desktop\GestureType;
use Pam\Desktop\UI\GestureDetector;

GestureDetector::make(
    GestureType::LongPress,
    Text::make('Hold me for half a second'),
)
    ->onBegin(function (GestureEvent $event): void {
        $this->status = 'Long press began';
    })
    ->onEnd(function (GestureEvent $event): void {
        $this->status = 'Long press ended';
    });

GestureDetector::make(
    GestureType::Swipe,
    Text::make('Swipe me in any direction'),
)->onEnd(function (GestureEvent $event): void {
    $this->status = sprintf('Swiped %d, %d', $event->translationX, $event->translationY);
});
```

### Types and how they map to a mouse and a trackpad

| `GestureType` | On desktop |
| --- | --- |
| `Tap` | A click: press and release without moving away. |
| `LongPress` | The primary button held for the minimum duration (500 ms by default) without moving away. |
| `Pan` | A drag with the primary button, recognised once the pointer travelled the minimum distance (8 units by default). |
| `Swipe` | A drag of at least the minimum distance (48 units by default), decided on release. |
| `Pinch` | A trackpad pinch, or Control + wheel, which zooms around the pointer. |
| `Rotation` | A two-finger rotation on the trackpad. |

A mouse is one pointer, so `Pinch` and `Rotation` never start from mouse buttons.

### Options

| Method | Meaning |
| --- | --- |
| `direction(GestureDirection)` | `Any`, `Left`, `Right`, `Up`, `Down`, `Horizontal` or `Vertical`. |
| `minimumDistance(float)` | Travel needed before a pan or a swipe is recognised. |
| `minimumDuration(int)` | Hold time in milliseconds. |
| `gestureEnabled(bool)` | Turns the detector off without removing it. |
| `onBegin`, `onUpdate`, `onEnd`, `onCancel` | The phases of the gesture. Updates are coalesced: at most one per drain of the host. |

### `GestureEvent`

| Property | Meaning |
| --- | --- |
| `type`, `state` | The `GestureType` and the `GestureState` (`Began`, `Changed`, `Ended`, `Cancelled`, `Failed`). |
| `x`, `y` | Pointer position inside the detector. |
| `pageX`, `pageY` | Pointer position in the window. |
| `translationX`, `translationY` | Travel since the gesture started. |
| `velocityX`, `velocityY` | Units per second. |
| `scale`, `rotation` | Of a pinch or a rotation. |
| `snapIndex`, `thresholdReached` | Results of a drag contract (below). |
| `nativeScale`, `nativeTranslationX`, `nativeTranslationY` | The pose of a native transform (below); `null` otherwise. |

## Drag contracts

A `Drag` attached to a pan detector moves a view with the pointer on one axis, drives other properties from that movement and settles on a snap point when released. All of that runs in the host; PHP receives the beginning, the end and the settle.

```php
use Pam\Desktop\Animation\Drag;
use Pam\Desktop\Animation\Spring;
use Pam\Desktop\GestureSettleEvent;

GestureDetector::make(GestureType::Pan, $sheet)
    ->drag(
        Drag::vertical()
            ->bounds(min: 0)
            ->snapPoints(0, '100%')
            ->threshold(120, velocity: 900)
            ->settle(new Spring(230, 22, 0.72))
            ->drive('', 'scale', [0, '50%'], [1, 0.955]),
    )
    ->onSettle(function (GestureSettleEvent $event): void {
        if ($event->snapIndex === 1) {
            $this->open = false;
        }
    });
```

| Method | Meaning |
| --- | --- |
| `Drag::horizontal()`, `Drag::vertical()` | The axis. |
| `bounds($min, $max)` | Limits of the translation relative to the resting position; `null` is unbounded. |
| `rubberBand($factor)` | Fraction of the movement applied past the bounds; 0 is a hard stop. |
| `target($nativeRef)` | The view that moves, named with `nativeRef()`; by default the detector's child. |
| `snapPoints(...$points)` | Resting positions, in units or as a percentage of the target's extent (`'100%'`). |
| `threshold($distance, $velocity)` | Crossing the distance from the starting snap, or flinging faster than the velocity, moves to the adjacent snap. |
| `settle($curve, $durationMs)` | The settle animation: a `Spring`, an `Easing` or a CSS timing function. |
| `settleAt($index, $curve, $durationMs)` | A different settle animation for one snap point. |
| `drive($ref, $property, $input, $output)` | Interpolates a property of the view named `$ref` (`''` is the target) from the translation. |
| `group($name)` | Only one drag of a group rests away from zero. |

`GestureDetector::snapTo($index, $request)` settles on a snap point from code; change `$request` to issue the same snap again.

## Swipeable rows

`Swipeable` is a ready-made drag contract: a row that slides sideways to reveal action panels behind it.

```php
use Pam\Desktop\UI\Swipeable;

Swipeable::make(
    Row::make(Text::make($message))
        ->style(new Style(height: 56, paddingLeft: 16, alignItems: Align::Center, backgroundColor: 0xFFFFFFFF)),
)
    ->rightActions(Row::make($archive, $delete), 160)
    ->group('inbox')
    ->onOpen(function (string $side) use ($message): void {
        $this->status = "Opened {$message} ({$side})";
    })
    ->onClose(function () use ($message): void {
        $this->status = "Closed {$message}";
    })
    ->toElement()
    ->key($message);
```

| Method | Meaning |
| --- | --- |
| `leftActions($panel, $width)`, `rightActions($panel, $width)` | The panel revealed by dragging right or left, and its width. |
| `group($name)` | Opening one row closes the others of the group. |
| `threshold($fraction, $velocity)` | How far or how fast the row must be dragged to open or close. |
| `closeRequest($serial)`, `openRequest($side, $serial)` | Close or open from code; change the serial to issue the request again. |
| `onOpen`, `onClose`, `onBegin` | `onOpen` receives `'left'` or `'right'`. |

The row content must paint an opaque background, so the panels stay hidden while it is closed. `toElement()` turns the row into an element so that it can take a `key()`.

## Native transforms

`nativeTransform()` lets the user pan, zoom and rotate the content of a detector with no round trip to PHP. Nest a pinch detector inside a pan detector to get both on the same content:

```php
GestureDetector::make(
    GestureType::Pan,
    GestureDetector::make(
        GestureType::Pinch,
        Image::make('asset://photo.jpg')->fit(ImageFit::Cover)->style(new Style(width: 400, height: 300)),
    )->nativeTransform(maximumScale: 5, focalZoom: true, bounds: true),
)
    ->nativeTransform(maximumScale: 5, bounds: true)
    ->onEnd(function (GestureEvent $event): void {
        $this->status = sprintf('Photo at scale %.2f', $event->nativeScale ?? 1.0);
    })
    ->cursor('grab')
    ->style(new Style(width: 400, height: 300, borderRadius: 16, overflow: Overflow::Hidden));
```

The user drags to pan, and zooms with a trackpad pinch or Control + wheel.

| Argument | Meaning |
| --- | --- |
| `minimumScale`, `maximumScale` | Limits of the zoom (1 and 4 by default). |
| `focalZoom` | Zooms around the pointer instead of the centre. |
| `bounds` | Keeps the scaled content covering the detector; a scale below 1 springs back on release. |
| `contentAspect` | Width divided by height of the content, when it is contained in the child instead of filling it. |
| `resetOnEnd` | The content returns to its resting pose when the gesture ends. |
| `resetKey` | Change it to reset the pose from code. |

## Related

- [Styling](styling.md) for the properties that are animated.
- [Media and web](media-and-web.md) for clocks and progress bars bound to a media player, which also run without PHP.
- [Architecture](architecture.md#what-runs-without-php) for how the frame clock works.
