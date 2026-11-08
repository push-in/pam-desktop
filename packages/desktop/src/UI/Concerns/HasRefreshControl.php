<?php

declare(strict_types=1);

namespace Pam\Desktop\UI\Concerns;

use Closure;
use Pam\Desktop\EventKind;
use Pam\Desktop\PropKey;
use Pam\Desktop\RefreshIndicatorSize;

/**
 * RN `refreshControl` straight on a ScrollView or list. Desktops have no
 * pull gesture: F5 or Ctrl/Cmd+R fires `onRefresh` on the visible refresh
 * host, and the host draws its own spinner over the content.
 * The scrollable is hosted in a RefreshControl automatically; its outer box
 * (size, flex, margins, position) sizes the host. `refreshing` is controlled:
 * set it true while loading and false to end the spinner.
 */
trait HasRefreshControl
{
    public function refreshing(bool $refreshing): self
    {
        return $this->withProperty(PropKey::Refreshing, $refreshing);
    }

    public function onRefresh(Closure $handler): self
    {
        return $this->withEvent(EventKind::Refresh, $handler);
    }

    /** Spinner colors. The desktop spinner uses the first one, unless a tint is set. */
    public function refreshColors(int ...$colors): self
    {
        return $this->withProperty(PropKey::RefreshColors, implode(',', $colors));
    }

    /** RN `tintColor`: the spinner color; wins over refreshColors(). */
    public function refreshTintColor(int $color): self
    {
        return $this->withProperty(PropKey::TintColor, $color);
    }

    public function refreshProgressBackgroundColor(int $color): self
    {
        return $this->withProperty(PropKey::RefreshProgressBackgroundColor, $color);
    }

    /** Spinner offset from the top, e.g. below a translucent header. */
    public function progressViewOffset(float $offset): self
    {
        return $this->withProperty(PropKey::RefreshProgressViewOffset, $offset);
    }

    public function refreshIndicatorSize(RefreshIndicatorSize $size): self
    {
        return $this->withProperty(PropKey::RefreshIndicatorSize, $size->value);
    }

    /** RN `RefreshControl.enabled`: false keeps the control but blocks new pulls. */
    public function refreshEnabled(bool $enabled = true): self
    {
        return $this->withProperty(PropKey::RefreshEnabled, $enabled);
    }
}
