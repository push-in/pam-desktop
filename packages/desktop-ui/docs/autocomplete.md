# Native asynchronous autocomplete

Register `DesktopUI::boot()` and place a `SelectProvider` around the form/modal. Pass its host name to `Autocomplete`. The editable value is controlled: update the owner field in `changed`, and set the control's `value` when accepting a suggestion.

```php
$input = new Autocomplete(
    value: $this->name,
    changed: fn (string $value) => $this->name = $value,
    selected: $this->selectItem(...),
    suggest: $this->suggestItems(...),
    host: $this->host,
);
```

The provider receives `(string $query, Closure $complete, Closure $failed)` and returns a cancellation closure for its active request. Complete with a list of `AutocompleteOption(value, label, detail, image, disabled)`; identifiers are integers or strings. The default limit is eight, with a hard maximum of twenty. Enforce the same limit at the API; the control rejects oversized results rather than loading an entire remote catalog. Images are rendered only for virtualized visible rows.

Queries start after 250 ms and require two characters by default. A replacement query, blur, Escape, selection, or unmount cancels the timer and request and invalidates late completions. Arrow keys skip disabled entries; Enter selects. Loading, empty results, failures with retry, and the anchored overlay render as native tags. In-flight state is excluded from hot reload. No network or application API is built into the control.

### Bounded option pages

Use `showImages: false` for compact text options, `minimumLength: 0` and
`searchOnFocus: true` to browse on demand. No query runs at construction or mount.
Focusing a selected control starts with an empty query while leaving its selected
label visible. The active query is kept separately, so previous/next pages use the
same filter; typing starts a new query normally.
An optional `pageProvider(query, page, complete, failed)` returns a cancellation
closure and completes with `AutocompletePage(options, hasMore)`. Pages replace the
current results; they never accumulate. Keep responses within `limit` (8 by
default). The native previous/next footer uses the same debounce, cancellation,
stale-response and pause lifecycle as text searches. Selected domain values belong
to the consumer independently of the current result page.

`inputStyle: new AutocompleteInputStyle(height: 50, fontSize: 16,
fontWeight: 700, paddingHorizontal: 16, radius: 8)` adjusts input geometry only.
Existing defaults and provider pagination/cancellation remain unchanged.
