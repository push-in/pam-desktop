<?php

declare(strict_types=1);

namespace Pam\Desktop\Modules;

/**
 * A binary value in a module payload: sent as opaque bytes (wire tag 5)
 * instead of UTF-8 text. Module results that carry bytes decode to plain
 * PHP strings.
 */
final readonly class WireBytes
{
    public function __construct(public string $bytes)
    {
    }
}
