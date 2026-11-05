<?php

declare(strict_types=1);

namespace Pam\Desktop;

final readonly class DeviceStatus
{
    public function __construct(
        public float $batteryLevel,
        public bool $charging,
        public NetworkType $networkType,
        public bool $expensiveNetwork,
        public bool $lowPowerMode,
    ) {
    }

    /** The system's power-saving mode. Alias of $lowPowerMode; always false on desktop. */
    public function powerSaveMode(): bool
    {
        return $this->lowPowerMode;
    }
}
