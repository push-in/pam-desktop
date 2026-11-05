<?php

declare(strict_types=1);

namespace Pam\Desktop;

use InvalidArgumentException;

final readonly class PerformanceBudget
{
    public function __construct(
        public int $coldStartMilliseconds = 1_500,
        public int $warmStartMilliseconds = 500,
        public int $idleMemoryMegabytes = 180,
        public int $idleCpuBasisPoints = 100,
        public int $ipcP95Microseconds = 5_000,
        public int $frameP95Microseconds = 16_667,
    ) {
        foreach (get_object_vars($this) as $name => $value) {
            if ($value < 1) {
                throw new InvalidArgumentException("Performance budget {$name} must be positive.");
            }
        }
    }

    /** @return array<string, int> */
    public function toArray(): array
    {
        return [
            'coldStartMilliseconds' => $this->coldStartMilliseconds,
            'warmStartMilliseconds' => $this->warmStartMilliseconds,
            'idleMemoryMegabytes' => $this->idleMemoryMegabytes,
            'idleCpuBasisPoints' => $this->idleCpuBasisPoints,
            'ipcP95Microseconds' => $this->ipcP95Microseconds,
            'frameP95Microseconds' => $this->frameP95Microseconds,
        ];
    }
}
