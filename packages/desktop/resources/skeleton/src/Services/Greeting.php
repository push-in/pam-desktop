<?php

declare(strict_types=1);

namespace App\Services;

/** Application logic lives in services: screens ask, services answer. */
final class Greeting
{
    public function forHour(int $hour): string
    {
        return match (true) {
            $hour < 5 => 'Good night',
            $hour < 12 => 'Good morning',
            $hour < 18 => 'Good afternoon',
            default => 'Good evening',
        };
    }
}
