<?php

declare(strict_types=1);

namespace Pam\Desktop\Canvas;

/**
 * Pointer events emitted by the native canvas view.
 */
enum CanvasEventKind: int
{
    case PointerDown = 1;
    case PointerMove = 2;
    case PointerUp = 3;
    case PointerCancel = 4;
}
