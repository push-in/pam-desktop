<?php

declare(strict_types=1);

namespace Pam\DesktopUi\Concerns;

use Closure;
use Pam\DesktopUi\SelectTheme;

/**
 * A control drawn with a {@see SelectTheme} given as a value or as a Closure.
 * The Closure is called again on every render, and a light/dark switch
 * renders every component again, so a theme built from the scheme or from
 * tokens follows it at once.
 */
trait FollowsSelectTheme
{
    /** The theme drawn by the current render. */
    public private(set) SelectTheme $theme;

    /** @var SelectTheme|Closure(): SelectTheme */
    private SelectTheme|Closure $themeSource;

    /** @param SelectTheme|(Closure(): SelectTheme)|null $theme */
    private function followTheme(SelectTheme|Closure|null $theme): void
    {
        $this->themeSource = $theme ?? new SelectTheme();
        $this->theme = SelectTheme::resolve($this->themeSource);
    }

    public function rendering(): void
    {
        $this->theme = SelectTheme::resolve($this->themeSource);
    }
}
