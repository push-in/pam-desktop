# Components

The interface of an application is a tree of elements returned by the `render()` method of a component. This page lists the elements that exist on desktop, with a short sample of each.

The samples are fragments of a `render()` method. They assume the classes are imported from `Pam\Desktop\UI` (elements), `Pam\Desktop` (`Style`, enums) and that `$this` is a component with the `#[State]` properties they mention. Every sample in this page has a complete, runnable counterpart under `examples/`.

## Conventions

- Elements are created with a static `make()` and are immutable: every method returns a modified copy.
- Containers take their children as arguments. `null` and `false` children are skipped, so conditions read naturally:

  ```php
  Column::make(
      Text::make('Always here'),
      $this->loading ? ActivityIndicator::make() : null,
  );
  ```

- Give elements built in a loop a stable `key()` so they keep their identity (and their native view) when the list changes:

  ```php
  $rows = [];
  foreach ($this->documents as $index => $name) {
      $rows[] = Text::make($name)->key($name.$index);
  }
  ```

- Sizes are logical units, independent of the display scale. Colors are `0xAARRGGBB` integers.
- Event handlers are closures. They may change `#[State]` properties; the component renders again after the handler returns.

## Layout

`Column` stacks its children vertically, `Row` horizontally. `View` is a plain box. All three take a [`Style`](styling.md) with flexbox properties.

```php
Row::make(
    Column::make(
        Text::make('Timer'),
        Text::make("{$this->seconds} s"),
    )->style(new Style(flexGrow: 1)),
    View::make()->style(new Style(width: 56, height: 56, borderRadius: 16, backgroundColor: 0xFF2563EB)),
)->style(new Style(padding: 16, gap: 12, alignItems: Align::Center));
```

`Spacer::make(8)` is an empty box of the given height. `Screen::make(...)` is the root box of a route in a navigator.

A box with `flexGrow: 1` takes the space its siblings leave. The root element of the examples uses it to fill the window:

```php
Column::make(/* … */)->style(new Style(flexGrow: 1, padding: 24, gap: 14, backgroundColor: 0xFFF3F4F6));
```

## Text

```php
Text::make('Selectable text')->style(new Style(fontSize: 24, fontWeight: 700, textColor: 0xFF12161C));

Text::make($this->description)
    ->numberOfLines(2)
    ->selectable()
    ->style(new Style(fontSize: 16, lineHeight: 25));
```

Text is laid out and drawn by the text engine of the operating system. `selectable()` lets the user select it with the pointer and copy it; see [Accessibility](accessibility.md#text-selection).

`Text::rich()` builds one paragraph from several runs, each with its own style:

```php
Text::rich(
    'Total: ',
    Text::make('R$ 42,00')->style(new Style(fontWeight: 700)),
);
```

## Buttons and pressables

`Button` is a labelled push button. Without a background, border or text color of your own it gets a neutral native look that follows light and dark mode and reacts to hover and press.

```php
Button::make('Increment')->onPress(function (): void {
    $this->count++;
});

Button::make('Delete')
    ->onPress($this->delete(...))
    ->style(new Style(backgroundColor: 0xFFDC2626, textColor: 0xFFFFFFFF, borderRadius: 8));
```

`Pressable` makes any content react to presses:

```php
Pressable::make(
    Text::make($name)->style(new Style(fontSize: 14)),
)
    ->onPress(function () use ($index): void {
        $this->selected = $index;
    })
    ->onLongPress($this->rename(...))
    ->cursor('pointer')
    ->tooltip("Open {$name}")
    ->style(new Style(paddingHorizontal: 12, height: 36, justifyContent: Justify::Center, borderRadius: 8));
```

| Method | Meaning |
| --- | --- |
| `onPress`, `onLongPress`, `onDoubleTap` | The three press gestures. A handler that declares a `PressEvent` parameter receives the pointer position. |
| `onPressIn`, `onPressOut` | The pointer went down or up on the element. |
| `pressedOpacity`, `pressedScale`, `pressedBackgroundColor` | Feedback while pressed, animated by the host. |
| `hitSlop` | Extends the area that reacts beyond the box. |
| `delayLongPress` | How long a press is held before `onLongPress` (500 ms by default). |

A focused pressable is activated with Enter or Space. Hover, tooltips, cursors and context menus work on every element; they are described in [Styling](styling.md) and [Desktop shell](desktop-shell.md).

## Inputs

`Input` is a native text control: typing, selection, the clipboard and input methods are the system's. PHP receives the text through `onChange`.

```php
Input::make($this->name)
    ->placeholder('Your name')
    ->onChange(function (string $value): void {
        $this->name = $value;
    })
    ->onSubmit($this->save(...))
    ->style(new Style(height: 44, paddingHorizontal: 12, borderRadius: 10, borderWidth: 1, borderColor: 0xFFD1D5DB, fontSize: 15));
```

| Method | Meaning |
| --- | --- |
| `multiline()` | A multi-line editor instead of a single-line field. |
| `secure()` | Password entry. |
| `maxLength(int)` | Longest text accepted. |
| `autoFocus()` | Takes keyboard focus when it appears. |
| `editable(false)` | Read-only: focusable and selectable, never modified. |
| `keyboard(KeyboardType)` | What the field is for: `Text`, `Email`, `Number`, `Phone`, `Decimal`, `Url`. |
| `onFocus`, `onBlur`, `onSubmit` | Focus changes and Enter. |
| `mask(pattern)` | Formats while typing; `#` stands for a digit. |
| `currency(...)` | Formats an amount with a prefix, a suffix and a locale. |

Masks are applied by the host as the user types, with no round trip to PHP:

```php
Input::make($this->phone)
    ->placeholder('(11) 98765-4321')
    ->mask('(##) #####-####')
    ->onChange(function (string $value): void {
        $this->phone = $value;
    });

Input::make($this->amount)
    ->currency(prefix: 'R$ ', locale: 'pt-BR')
    ->onChange(function (string $value): void {
        $this->amount = $value;
    });
```

`TextInput::make()` is an alias of `Input::make()`.

## Switch and activity indicator

Both are drawn and animated by the host.

```php
Toggle::make($this->notifications)->onToggle(function (string $value): void {
    $this->notifications = $value === '1';
});

ActivityIndicator::make($this->syncing);
```

`onToggle` receives `'1'` or `'0'`. `ActivityIndicator::make(false)` stops and hides the indicator.

## Images

```php
Image::make('https://picsum.photos/id/1015/600/400')
    ->fit(ImageFit::Cover)
    ->style(new Style(width: 200, height: 130, borderRadius: 14));

ImageBackground::make(
    'asset://assets/header.jpg',
    Text::make('Children are laid out above the picture'),
)->style(new Style(height: 150, padding: 20));
```

Sources, the cache, placeholders and load events are in [Media and web](media-and-web.md#image).

## Scroll views

`Scroll` (alias `ScrollView`) scrolls one child with the native scroll view of the platform: wheel, trackpad, scrollbars and keyboard are native.

```php
Scroll::make(
    Column::make(...$paragraphs)->style(new Style(padding: 24, gap: 12)),
)
    ->onScroll(function (string $offset): void {
        $this->offset = (float) $offset;
    })
    ->style(new Style(flexGrow: 1));
```

| Method | Meaning |
| --- | --- |
| `horizontal()` | Scrolls horizontally. |
| `scrollEnabled(false)` | Disables user scrolling. |
| `showsIndicator(false)` | Hides the scrollbars. |
| `scrollRequest($serial, targetOffset: $y, animated: true)` | Scrolls from code. The request runs each time `$serial` changes. |
| `onScroll`, `onEndReached` | The offset changed; the end of the content is near. |
| `refreshing(bool)`, `onRefresh` | Refresh. On desktop the user asks for it with F5 or Ctrl+R (Cmd+R). |

A child marked `stickyHeader()` stays pinned to the top of the scroll view once scrolled past, until the next sticky sibling pushes it away.

### Refresh

There is no pull gesture on desktop. F5 or Ctrl+R (Cmd+R) asks the visible scroll view or list that has `onRefresh` to refresh: the handler runs and the host shows a spinner at the top of the view while `refreshing` is `true`.

| Method | Meaning |
| --- | --- |
| `refreshEnabled(false)` | The keys no longer ask for a refresh. `refreshing(true)` still shows the spinner. |
| `refreshTintColor($color)` | The color of the spinner. |
| `refreshColors(...$colors)` | The first color is the color of the spinner, unless a tint is set. |
| `refreshProgressBackgroundColor($color)` | A disc of that color behind the spinner. |
| `refreshIndicatorSize(RefreshIndicatorSize::Large)` | A larger spinner. |
| `progressViewOffset($offset)` | Moves the spinner down, for example below a header. |

`Scroll`, `VirtualizedList`, `FlatList` and `SectionList` take these methods.

## Lists

`VirtualizedList` shows long lists. PHP builds only the rows near the viewport and the host asks for more as the user scrolls, so a list of thousands of items costs what is on screen.

```php
VirtualizedList::of(
    range(1, 2000),
    static fn (int $row): Row => Row::make(
        Text::make("Row number {$row}"),
    )->style(new Style(height: 56, paddingHorizontal: 14, alignItems: Align::Center)),
    static fn (int $row): string => "row-{$row}",
)
    ->rowHeight(56)
    ->onEndReached(function (): void {
        $this->loadMore();
    })
    ->style(new Style(flexGrow: 1));
```

`of($items, $row, $key)` takes the data, a closure that builds the row for one item (it also receives the index) and a closure that returns a stable key. Without a key closure the key is the item's `key` or `id`, then its index.

| Method | Meaning |
| --- | --- |
| `rowHeight(float)` | Height of a row, or the estimate for rows without an authored height. |
| `columns(int)` | A grid with that many columns. `VirtualGrid::of($columns, $items, $cell)` is a shortcut. |
| `horizontal()` | A horizontal list. |
| `inverted()` | Row 0 rests at the bottom and older rows go up, as in a conversation. |
| `initialScrollIndex(int)` | The row the list opens at. |
| `scrollRequest($serial, targetOffset: $y)` | Scrolls from code, as in `Scroll`. |
| `onScroll`, `onEndReached` | As in `Scroll`. |
| `refreshing(bool)`, `onRefresh` | As in `Scroll`. |

`VirtualizedList::make(...)` takes ready-made elements (or closures that return them) instead of data. `FlatList::make([...])` and `SectionList::make(['Title' => [...]])` show plain strings, the second one grouped under headers.

`examples/lists` is a 2000-row list with programmatic scrolling; `examples/chat` is an inverted list.

## Modal and bottom sheet

A `Modal` is presented above the window content, with a backdrop. It is part of the tree all the time; its second argument says whether it is visible.

```php
Modal::make(
    Column::make(
        Text::make('Delete this project?'),
        Button::make('Keep')->onPress(function (): void {
            $this->dialog = false;
        }),
    )->style(new Style(width: 340, padding: 20, gap: 12, borderRadius: 16, backgroundColor: 0xFFFFFFFF)),
    $this->dialog,
    ModalPresentation::Dialog,
)
    ->animationType(ModalAnimationType::Fade)
    ->onRequestClose(function (): void {
        $this->dialog = false;
    });
```

`onRequestClose` runs when the user presses Escape, asks to go back or clicks the backdrop. The host starts the exit animation at once and then tells PHP, which must set the state that hides the modal.

`ModalPresentation` is `Dialog`, `FullScreen` or `Sheet`.

A `BottomSheet` is a modal anchored to the bottom edge of the window. Its snap points are fractions of the available height:

```php
BottomSheet::make(
    Column::make(
        Text::make('Share with'),
        Button::make('Done')->onPress(function (): void {
            $this->sheet = false;
        }),
    )->style(new Style(padding: 24, gap: 12, backgroundColor: 0xFFFFFFFF)),
    [0.45],
    0,
    $this->sheet,
)->onDismiss(function (): void {
    $this->sheet = false;
});
```

For a separate native window instead of an overlay, use `UI\Window`: see [Desktop shell](desktop-shell.md#secondary-windows).

## Navigation

A `Navigator` holds a stack of routes. Each route is a closure that returns the screen; the navigator renders the top of the stack and animates between routes.

```php
use Pam\Desktop\Navigation\NavigationPresentation;
use Pam\Desktop\Navigation\NavigationTransition;
use Pam\Desktop\Navigation\Navigator;
use Pam\Desktop\Navigation\ScreenOptions;

final class Mail extends Component
{
    private Navigator $navigator;

    #[State]
    public string $query = '';

    public function mount(): void
    {
        $this->navigator = new Navigator(
            initialRoute: 'inbox',
            routes: [
                'inbox' => fn (): Renderable => Screen::make(Column::make(
                    Button::make('Open a message')->onPress(fn () => $this->navigator->push('message')),
                    Button::make('Filters')->onPress(fn () => $this->navigator->push('filters')),
                )->style(new Style(flexGrow: 1, paddingTop: 72, paddingHorizontal: 20, gap: 12))),
                'message' => fn (): Renderable => Screen::make(
                    Text::make('Invoice 204')->style(new Style(paddingTop: 72, paddingHorizontal: 20)),
                ),
                'filters' => fn (): Renderable => Screen::make(Column::make(
                    Button::make('Done')->onPress(fn () => $this->navigator->pop()),
                )->style(new Style(flexGrow: 1, padding: 24))),
            ],
            restorePersistedState: false,
            screenOptions: [
                'inbox' => new ScreenOptions(title: 'Inbox', headerShown: true),
                'message' => new ScreenOptions(title: 'Invoice 204', headerShown: true),
                'filters' => new ScreenOptions(
                    presentation: NavigationPresentation::FormSheet,
                    animation: NavigationTransition::SlideFromBottom,
                    sheetAllowedDetents: [0.45, 1.0],
                ),
            ],
        );
    }

    public function render(): Renderable
    {
        return $this->navigator->render();
    }
}
```

`push($route)` adds a route, `pop()` removes the top one, `replace()` swaps it and `popToTop()` returns to the first. The user goes back with Alt+Left on Linux and Windows, Cmd+[ on macOS, or the back button of the mouse.

By default a navigator stores its stack in the state directory and restores it at the next launch; `restorePersistedState: false` turns that off.

### Header

With `headerShown: true` the host draws a bar over the top of the route: the title, a back button when there is a route below, and optionally a search field. The route content is laid out under the bar, so the screens above leave `paddingTop: 72` for it.

```php
new ScreenOptions(
    title: 'Search',
    headerShown: true,
    headerBackgroundColor: 0xFF2F6BFF,
    headerTintColor: 0xFFFFFFFF,
    headerSearchEnabled: true,
    headerSearchPlaceholder: 'Search messages',
    onHeaderSearchChange: function (string $query): void {
        $this->query = $query;
    },
);
```

### Form sheet

A route with `presentation: NavigationPresentation::FormSheet` covers part of the route below, which stays visible. `sheetAllowedDetents` lists the heights it may rest at, as fractions of the window.

### Shared elements

An element with the same `sharedTransition()` tag on two routes flies from one to the other when the navigator moves between them, above both routes:

```php
// On the list route
Column::make(Text::make('Invoice 204'))
    ->sharedTransition('invoice')
    ->style(new Style(width: 220, padding: 14, borderRadius: 14, backgroundColor: 0xFF2F6BFF));

// On the detail route
Column::make(Text::make('Invoice 204')->style(new Style(fontSize: 28)))
    ->sharedTransition('invoice', SharedTransitionStyle::spring())
    ->style(new Style(height: 180, padding: 24, backgroundColor: 0xFF2F6BFF));
```

The host does it all: PHP runs no code per frame.

- Between two images, one picture flies and is resized on the way, with its corner radius.
- Between any other pair, the host copies both elements with what they contain. The copies are stretched to the frame in flight and the one that leaves fades into the one that arrives. `resize()` with `SharedTransitionResizeMode::None` or `Clip` moves them without stretching, and `crossFade(false)` flies only the arriving one.
- The flight lasts as long as the navigation, or what the `SharedTransitionStyle` of the element on the top route says; `returning()` sets another curve for the way back.

An element only leaves from where it can be seen: one that is mostly scrolled out appears with its route. So does one with more than 96 elements inside. Text fields, players, web pages and scrolling content inside a flying element are drawn as their box during the flight.

`examples/navigation` shows the header, the search field, a form sheet, and a picture and a card that fly to their routes.

## Tabs

A `TabNavigator` shows one scene per tab and a bar drawn by the host. Only the selected scene is visible; the others stay mounted once they were opened.

```php
use Pam\Desktop\Navigation\NavigationTab;
use Pam\Desktop\Navigation\TabNavigator;

App::run(new TabNavigator('inbox', [
    new NavigationTab('inbox', 'Inbox', fn (): Renderable => Screen::make(Text::make('Inbox'))),
    new NavigationTab('sent', 'Sent', fn (): Renderable => Screen::make(Text::make('Sent')), badge: '3'),
    new NavigationTab('settings', 'Settings', new SettingsScreen()),
]));
```

The content of a tab is an element, a component or a closure that returns one. `select('sent')` changes the tab from code and `selectedTab()` reads the current one.

With the default `TabPresentation::Adaptive` the bar is a rail on the side when the window is at least 840 units wide and a bar at the bottom otherwise. The bar is made of ordinary scene nodes, so it takes keyboard focus like the rest of the interface.

## Elements described elsewhere

| Element | Document |
| --- | --- |
| `Image`, `ImageBackground`, `MediaPlayer`, `WebView`, `Canvas`, `DrawingCanvas` | [Media and web](media-and-web.md) |
| `GestureDetector`, `Swipeable` | [Animation and gestures](animation-and-gestures.md) |
| `InteractionRegion` (drag and drop), `UI\Window` (secondary windows) | [Desktop shell](desktop-shell.md) |
