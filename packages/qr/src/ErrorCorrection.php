<?php

declare(strict_types=1);
namespace Pam\Desktop\Qr;
use BaconQrCode\Common\ErrorCorrectionLevel;
enum ErrorCorrection: int
{
    case Low = 1;
    case Medium = 2;
    case Quartile = 3;
    case High = 4;
    /** Explicit adapter: the established encoder uses its own non-sequential bit representation. */
    public function encoder(): ErrorCorrectionLevel
    {
        return match ($this) { self::Low => ErrorCorrectionLevel::L(), self::Medium => ErrorCorrectionLevel::M(), self::Quartile => ErrorCorrectionLevel::Q(), self::High => ErrorCorrectionLevel::H() };
    }
}
