<?php

declare(strict_types=1);

namespace Pam\DesktopUi;

/** One queued toast. `life` in ms; null keeps it until closed (PrimeVue sticky). */
final readonly class ToastMessage
{
    public function __construct(
        public int $id,
        public ToastSeverity $severity,
        public string $summary,
        public string $detail = '',
        public ?int $life = 3000,
        public bool $closable = true,
    ) {
        if ($life !== null && ($life < 500 || $life > 120_000)) {
            throw new \InvalidArgumentException('Toast life must be between 500 and 120000 ms.');
        }
        if (mb_strlen($summary) > 200 || mb_strlen($detail) > 1000) {
            throw new \InvalidArgumentException('Toast text is too long.');
        }
    }
}
