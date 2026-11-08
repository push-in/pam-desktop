<?php

declare(strict_types=1);

namespace App\Lib;

/** Small helpers shared by screens and services. */
final class Plural
{
    public static function of(int $count, string $one, string $many): string
    {
        return $count.' '.($count === 1 ? $one : $many);
    }
}
