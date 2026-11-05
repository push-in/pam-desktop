<?php

declare(strict_types=1);

namespace Pam\Desktop;

enum AccessibilityLiveRegion: int
{
    case None = 1;
    case Polite = 2;
    case Assertive = 3;
}
