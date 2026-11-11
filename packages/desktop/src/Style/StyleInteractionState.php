<?php

declare(strict_types=1);

namespace Pam\Desktop\Style;

enum StyleInteractionState: int
{
    case Pressed = 1;
    case Focused = 2;
    case Hovered = 3;
    case Disabled = 4;
    case Checked = 5;
    case Selected = 6;
    case Active = 7;
    case Loading = 8;
    case Error = 9;
    case FocusWithin = 10;
    /** `:focus-visible`: focused by the keyboard (host-evaluated). */
    case FocusVisible = 11;
    /** `.group:hover .node`: the nearest hover group above is hovered (host-evaluated). */
    case GroupHover = 12;
}
