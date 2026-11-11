<?php

declare(strict_types=1);

namespace Pam\Desktop\UI;

use Closure;
use InvalidArgumentException;
use Pam\Desktop\Element;
use Pam\Desktop\EventKind;
use Pam\Desktop\Internal\Wire;
use Pam\Desktop\NodeKind;
use Pam\Desktop\PropKey;

/**
 * A VT terminal (xterm-256color) drawn by the host. It shows a channel of the
 * host's terminal hub, such as the `channel()` of an SSH shell from
 * `pushinbr/pam-desktop-ssh`: output goes from the network to the screen and
 * keystrokes back without passing through PHP.
 *
 * Size it like any box (it fills its frame with whole cells); the grid size
 * is sent to the remote end and reported by onResize().
 */
final class Terminal extends Element
{
    public const DEFAULT_FONT_FAMILY = "'JetBrains Mono', 'DejaVu Sans Mono', 'Cascadia Mono', Menlo, Consolas, monospace";
    public const MAX_SCROLLBACK = 100_000;

    public static function make(string $channel): self
    {
        if ($channel === '' || strlen($channel) > 64) {
            throw new InvalidArgumentException('A Terminal channel has 1 to 64 bytes.');
        }

        return (new self(NodeKind::Terminal))
            ->withProperty(PropKey::TerminalChannel, $channel)
            ->withProperty(PropKey::FontFamily, self::DEFAULT_FONT_FAMILY)
            ->withProperty(PropKey::FontSize, 13.0)
            ->withProperty(PropKey::Enabled, true);
    }

    public function channel(string $channel): self
    {
        if ($channel === '' || strlen($channel) > 64) {
            throw new InvalidArgumentException('A Terminal channel has 1 to 64 bytes.');
        }

        return $this->withProperty(PropKey::TerminalChannel, $channel);
    }

    /** A monospace CSS font-family list. */
    public function font(string $family, float $size = 13.0, float $lineHeight = 0.0): self
    {
        $element = $this->withProperty(PropKey::FontFamily, $family)
            ->withProperty(PropKey::FontSize, max(4.0, min(96.0, $size)));

        return $lineHeight > 0 ? $element->withProperty(PropKey::LineHeight, $lineHeight) : $element;
    }

    /** Lines kept above the screen (5000 by default, at most 100000). */
    public function scrollback(int $lines): self
    {
        return $this->withProperty(PropKey::TerminalScrollback, max(0, min(self::MAX_SCROLLBACK, $lines)));
    }

    /**
     * Colours as 0xAARRGGBB integers or `#rrggbb` strings; null keeps the
     * default dark theme's.
     *
     * @param list<int|string>|null $palette The 16 ANSI colours (black … bright white).
     */
    public function theme(int|string|null $background = null, int|string|null $foreground = null, int|string|null $cursor = null, int|string|null $selection = null, ?array $palette = null): self
    {
        $element = $this;
        foreach ([
            [PropKey::BackgroundColor, $background],
            [PropKey::TextColor, $foreground],
            [PropKey::TerminalCursorColor, $cursor],
            [PropKey::TerminalSelectionColor, $selection],
        ] as [$key, $value]) {
            if ($value !== null) {
                $element = $element->withProperty($key, self::color($value));
            }
        }
        if ($palette !== null) {
            $element = $element->withProperty(PropKey::TerminalPalette, self::palette($palette));
        }

        return $element;
    }

    /** @param Closure(int $cols, int $rows): mixed $handler */
    public function onResize(Closure $handler): self
    {
        return $this->withEvent(EventKind::TerminalResize, static function (string $payload) use ($handler): mixed {
            $values = Wire::decodeMap($payload);

            return $handler((int) ($values['cols'] ?? 0), (int) ($values['rows'] ?? 0));
        });
    }

    /** @param Closure(string $title): mixed $handler The window title the remote end set (OSC 0/2). */
    public function onTitle(Closure $handler): self
    {
        return $this->withEvent(EventKind::TerminalTitle, static function (string $payload) use ($handler): mixed {
            return $handler((string) (Wire::decodeMap($payload)['title'] ?? ''));
        });
    }

    /** @param Closure(int $exitCode): mixed $handler The channel ended (-1 when no status was sent). */
    public function onExit(Closure $handler): self
    {
        return $this->withEvent(EventKind::TerminalExit, static function (string $payload) use ($handler): mixed {
            return $handler((int) (Wire::decodeMap($payload)['exitCode'] ?? -1));
        });
    }

    /** @param Closure(): mixed $handler */
    public function onBell(Closure $handler): self
    {
        return $this->withEvent(EventKind::TerminalBell, static fn (string $payload): mixed => $handler());
    }

    /** @internal */
    public static function color(int|string $value): int
    {
        if (is_int($value)) {
            return $value & 0xFFFFFFFF;
        }
        if (preg_match('/^#([0-9a-fA-F]{6}|[0-9a-fA-F]{8})$/D', $value, $match) !== 1) {
            throw new InvalidArgumentException('Terminal colours are #rrggbb, #aarrggbb or 0xAARRGGBB.');
        }
        $hex = $match[1];

        return strlen($hex) === 6 ? 0xFF000000 | (int) hexdec($hex) : (int) hexdec($hex);
    }

    /**
     * @internal
     * @param array<array-key, mixed> $palette
     */
    public static function palette(array $palette): string
    {
        if (count($palette) > 16) {
            throw new InvalidArgumentException('A terminal palette has at most 16 colours.');
        }
        $entries = [];
        foreach (array_values($palette) as $value) {
            if (!is_int($value) && !is_string($value)) {
                throw new InvalidArgumentException('Terminal palette colours are integers or #rrggbb strings.');
            }
            $entries[] = sprintf('#%06x', self::color($value) & 0xFFFFFF);
        }

        return implode(',', $entries);
    }
}
