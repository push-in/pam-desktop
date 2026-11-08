<?php

declare(strict_types=1);

namespace Pam\DesktopUi;

/** PrimeVue Toast severities, as sequential integer variants. */
enum ToastSeverity: int
{
    case Success = 1;
    case Info = 2;
    case Warn = 3;
    case Error = 4;
    case Secondary = 5;
    case Contrast = 6;

    public function icon(): string
    {
        return match ($this) {
            self::Success => 'check',
            self::Info, self::Secondary, self::Contrast => 'info-circle',
            self::Warn => 'exclamation-triangle',
            self::Error => 'times-circle',
        };
    }

    /** Accent (text/icon) colour of the dark Aura preset. */
    public function accent(): int
    {
        return match ($this) {
            self::Success => 0xFF4ADE80,
            self::Info => 0xFF38BDF8,
            self::Warn => 0xFFFACC15,
            self::Error => 0xFFF87171,
            self::Secondary => 0xFFD4D4D8,
            self::Contrast => 0xFF18181B,
        };
    }

    public function surface(): int
    {
        return match ($this) {
            self::Success => 0x2922C55E,
            self::Info => 0x290EA5E9,
            self::Warn => 0x29EAB308,
            self::Error => 0x29EF4444,
            self::Secondary => 0xFF27272A,
            self::Contrast => 0xFFFFFFFF,
        };
    }

    public function border(): int
    {
        return match ($this) {
            self::Success => 0x3D22C55E,
            self::Info => 0x3D0EA5E9,
            self::Warn => 0x3DEAB308,
            self::Error => 0x3DEF4444,
            self::Secondary => 0xFF3F3F46,
            self::Contrast => 0xFFFFFFFF,
        };
    }
}
