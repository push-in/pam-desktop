# pushinbr/pam-desktop

The PHP side of PAM Desktop: reactive components, styles, navigation,
animation and the system modules of a native Linux, macOS and Windows
application. The interface is drawn by the platform toolkit (GTK 4, AppKit,
Win32 with Direct2D), not by a web view.

```php
<?php

use Pam\Desktop\App;
use Pam\Desktop\Attributes\State;
use Pam\Desktop\Component;
use Pam\Desktop\UI\Button;
use Pam\Desktop\UI\Column;
use Pam\Desktop\UI\Text;

require __DIR__.'/vendor/autoload.php';

final class Counter extends Component
{
    #[State]
    public int $count = 0;

    public function render(): Column
    {
        return Column::make(
            Text::make("Count: {$this->count}"),
            Button::make('Add one')->onPress(fn () => $this->count++),
        );
    }
}

App::run(new Counter());
```

Create and run an application with the [PAM CLI](https://github.com/push-in/pam),
which brings PHP 8.5 and Composer:

```bash
pam init my-app --template desktop   # writes the application and installs this package
cd my-app && pam desktop dev         # runs with hot reload
pam desktop build                    # bundles the application and its installers
```

`pam desktop` runs the native host of the installed version for the current
system: it downloads it once, verifies its SHA-256 and keeps it in the user
cache. `PAM_DESKTOP_HOST` points it at a host built from source instead.

The documentation lives in the [repository](https://github.com/push-in/pam-desktop/tree/main/docs).
