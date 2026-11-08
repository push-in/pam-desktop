<?php

declare(strict_types=1);
namespace Pam\Desktop;

use InvalidArgumentException;

/** Bounded native fractional grid tracks, serialized as minimum,weight pairs. */
final readonly class GridTracks
{
    /** @param list<array{float,float}> $tracks */
    private function __construct(private array $tracks) {}
    public static function fromCss(string $css): self
    {
        $css = trim($css);
        if (strlen($css) > 8192) throw new InvalidArgumentException('Grid tracks exceed 8192 bytes.');
        if (preg_match('/^repeat\(\s*(\d+)\s*,\s*(.+)\)$/iD', $css, $repeat)) {
            $count = (int) $repeat[1];
            if ($count < 1 || $count > 64) throw new InvalidArgumentException('Grid supports 1 to 64 tracks.');
            $css = implode(' ', array_fill(0, $count, trim($repeat[2])));
        }
        preg_match_all('/minmax\([^)]*\)|[^\s]+/', $css, $parts);
        $tracks = [];
        foreach ($parts[0] as $track) {
            if (preg_match('/^(\d+(?:\.\d+)?|\.\d+)fr$/iD', $track, $match)) $tracks[] = [0.0, (float) $match[1]];
            elseif (preg_match('/^minmax\(\s*(\d+(?:\.\d+)?)(?:px|dp)?\s*,\s*(\d+(?:\.\d+)?|\.\d+)fr\s*\)$/iD', $track, $match)) $tracks[] = [(float) $match[1], (float) $match[2]];
            else throw new InvalidArgumentException('Grid tracks require positive fr or minmax(non-negative px, positive fr).');
        }
        self::validate($tracks);
        return new self($tracks);
    }
    public static function fromWire(string $wire): self
    {
        if (strlen($wire) > 8192) throw new InvalidArgumentException('Grid tracks exceed 8192 bytes.');
        $tracks = [];
        foreach (explode(';', $wire) as $track) {
            $pair = explode(',', $track);
            if (count($pair) !== 2 || !is_numeric($pair[0]) || !is_numeric($pair[1])) throw new InvalidArgumentException('Invalid grid track wire value.');
            $tracks[] = [(float) $pair[0], (float) $pair[1]];
        }
        self::validate($tracks);
        return new self($tracks);
    }
    /** @param list<array{float,float}> $tracks */
    private static function validate(array $tracks): void
    {
        if ($tracks === [] || count($tracks) > 64) throw new InvalidArgumentException('Grid supports 1 to 64 tracks.');
        foreach ($tracks as [$minimum, $weight]) if (!is_finite($minimum) || !is_finite($weight) || $minimum < 0 || $minimum > 1_000_000 || $weight <= 0 || $weight > 1_000_000) throw new InvalidArgumentException('Invalid grid track bounds.');
    }
    public function count(): int { return count($this->tracks); }
    public function toWire(): string
    {
        return implode(';', array_map(static fn (array $pair): string => $pair[0].','.$pair[1], $this->tracks));
    }
}
