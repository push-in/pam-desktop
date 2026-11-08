<?php

declare(strict_types=1);
namespace Pam\DesktopUi;
use DateTimeImmutable;
use InvalidArgumentException;
final readonly class DateSelection
{
    public function __construct(public DateTimeImmutable $start, public ?DateTimeImmutable $end = null)
    {
        if ($end !== null && $end < $start) throw new InvalidArgumentException('The range end must not precede its start.');
    }
}
