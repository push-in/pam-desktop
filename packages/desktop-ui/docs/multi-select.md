# Native multi-select

`MultiSelect` is the multi-value companion to `Autocomplete`. It uses the
`SelectProvider` at the current window or dialog root, shows one bounded remote
page at a time, and keeps selected options independently of the active query or
page.

```php
$tags = new MultiSelect(
    selectedOptions: [],
    changed: $this->selectionChanged(...),
    pageProvider: $this->tagPage(...),
    host: $this->host,
    label: 'Add tags',
    limit: 8,
    minimumLength: 0,
    searchOnOpen: true,
);
```

The provider receives `(query, page, complete, failed)` and returns a
cancellation closure. Complete with `AutocompletePage` when there is a next
page. The current result page is replaced, while selected `AutocompleteOption`
objects remain available to the owner. Rows stay open after selection so users
can choose several results, including items from different pages. The owner can
commit those values and call `replaceSelection([])` to clear the temporary
selection.

Typing cancels the previous request immediately; completion callbacks from an
older query are ignored. Search is debounced, results are bounded to 20 (8 by
default), and the control cancels its timer and active request on dismissal,
pause, and unmount. It provides keyboard selection, selected-state semantics,
loading, empty, retryable error, and outside-dismiss behavior. It does not own
network, domain validation, or persistence.
