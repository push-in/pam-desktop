<?php

declare(strict_types=1);
namespace Pam\DesktopUi;

/** Trigger appearance; calendar behavior is independent of presentation. */
enum DatePickerPresentation: int
{
    case Button = 1;
    case Field = 2;
}
