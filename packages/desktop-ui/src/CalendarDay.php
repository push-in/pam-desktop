<?php

declare(strict_types=1);
namespace Pam\DesktopUi;
final readonly class CalendarDay
{
    public function __construct(public string $date, public string $label, public bool $currentMonth, public bool $selected, public bool $enabled) {}
}
