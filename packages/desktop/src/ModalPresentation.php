<?php

declare(strict_types=1);

namespace Pam\Desktop;

enum ModalPresentation: int
{
    case FullScreen = 1;
    case Dialog = 2;
    case Sheet = 3;
    /**
     * A window layer (the DesktopUI toast stack): drawn above every other
     * modal, with no backdrop; Escape and the backdrop never close it, and
     * presses outside its content reach what is below. Give its content
     * `pointerEvents: box-none`.
     */
    case Overlay = 4;
}
