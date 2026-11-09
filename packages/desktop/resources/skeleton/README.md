# @NAME@

A native desktop application built with [PAM Desktop](https://github.com/push-in/pam-desktop).

```bash
pam composer install
pam desktop dev
```

| Task | Command |
| --- | --- |
| Check every template, stylesheet and route | `pam desktop check` |
| Run the test | `pam composer test` |
| Bundle and build installers | `pam desktop build` (`--format deb`, `appimage`, `rpm`, …) |

`dev` finds the project from any of its folders, opens the window and reloads it on every save. Errors show where they happened, with the source and the stack.

| Path | What goes there |
| --- | --- |
| `index.php` | Boots the application. |
| `pam-desktop.json` | Identity, window and packaging. |
| `src/Navigation` | Routes (`AppRoute`) and the router (`AppRouter`). |
| `src/Screens` | One folder per screen: a `.pam.php` component with template and styles. |
| `src/Services` | Application logic the screens call. |
| `src/Lib` | Small shared helpers. |
| `src/app.css` | Styles shared by every screen. |
| `tests` | `composer test`. |
