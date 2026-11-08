# ProgressButton

A controlled native action that retains `label` while `loading` displays an
`ActivityIndicator`. Supply `enabled`, `backgroundColor`, `textColor`, and
`testId`, then listen to `on:press`. Loading and disabled actions reject
activation, and the busy state is exposed to accessibility. Disabled idle
actions use reduced opacity; a busy action keeps full opacity so explicit busy
colors and its label remain legible.

Optional `loadingBackgroundColor` and `loadingTextColor` set busy-state colors.
`loadingIndicatorColor` controls the indicator independently; when omitted, it
uses `loadingTextColor`, then `textColor`. These values use the same ARGB integer
format as PAM Desktop color properties. There is no PHP polling or animation
timer; the native host owns indicator animation and its lifecycle.

```html
<ProgressButton label="Sign in" testId="sign-in" :enabled="canSubmit()" :loading="$busy"
    backgroundColor="#2563EB" textColor="#FFFFFF" on:press="submit" />
```

`packages/desktop-ui/tests/progress-button.php` covers busy/disabled semantics
and busy visual properties. The loading state was checked on Linux; macOS and
Windows runtime captures remain pending.
