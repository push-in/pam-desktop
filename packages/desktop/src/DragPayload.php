<?php

declare(strict_types=1);

namespace Pam\Desktop;

use Pam\Desktop\Internal\Wire;

final readonly class DragPayload
{
    public function __construct(public int $type, public string $data)
    {
    }
    public static function fromPayload(string $payload): self
    {
        $map = Wire::decodeMap($payload);
        $type = $map['type'] ?? null;
        $data = $map['data'] ?? null;
        if (!is_int($type) || $type < 1 || !is_string($data) || strlen($data) > 512) {
            throw new \UnexpectedValueException('Invalid native drag payload.');
        }return new self($type, $data);
    }
}
