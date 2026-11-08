# Toast

For one stack per window, drawn above modals and kept across navigation, use
`Toast::service()->success('Saved')` (see `docs/desktop-ui.md`, "The window
toast service"). The component below is a stack bound to one screen.

A PrimeVue-style toast stack drawn in a Portal over `host` (a
`SelectProvider` or any portal host): severity colours, position, auto-dismiss
after `life`, a close button and a bounded queue. Timers stop while the screen
is paused and resume with the remaining time.

```php
$this->toast = new Toast(host: $this->host, position: ToastPosition::TopRight);
$this->toast->success('Saved', 'The product was updated.');
```

```html
<SelectProvider :host="$host">
    …
    <Slot :content="$toast" />
</SelectProvider>
```

| Argument | Default | Meaning |
| --- | --- | --- |
| `position` | `TopRight` | Corner or edge of the stack. |
| `limit` | 5 | Messages kept; the oldest leaves first. |
| `width`, `offset` | 400, 20 | Width of a message and its distance from the window edge. |
| `fontFamily` | `system-ui` | A CSS font list; `system-ui` is the platform's interface font. |
| `backdropBlur` | 1.5 | Blur, in px, of what is behind the translucent surface (CSS `backdrop-filter: blur()`, as PrimeVue Aura). 0 turns it off. |

The severity glyphs come from the `PrimeIcons` icon font, which the
application registers (`Icon::register('PrimeIcons', 'asset://…')`).
`packages/desktop-ui/tests/toast.php` covers the queue, timers and defaults.
