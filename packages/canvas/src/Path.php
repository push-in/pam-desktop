<?php

declare(strict_types=1);

namespace Pam\Desktop\Canvas;

use InvalidArgumentException;
use Pam\Desktop\Canvas\Internal\Numbers;
use Stringable;

/**
 * Immutable builder for the compact path specification used by the `Path`
 * and `GradientPath` commands.
 *
 * The spec is SVG-like with absolute coordinates and only five verbs:
 * `M x y`, `L x y`, `C x1 y1 x2 y2 x y`, `Q cx cy x y` and `Z`.
 *
 * ```php
 * $path = Path::start(0, 10)->lineTo(20, 5)->curveTo(30, 0, 40, 0, 50, 10)->close();
 * $path->spec(); // "M0 10 L20 5 C30 0 40 0 50 10 Z"
 * ```
 */
final readonly class Path implements Stringable
{
    public const int MAX_SPEC_LENGTH = 4096;

    /** @var array<string, int> arguments expected by each verb */
    private const array ARITY = ['M' => 2, 'L' => 2, 'C' => 6, 'Q' => 4, 'Z' => 0];

    /**
     * @param list<string> $segments
     */
    private function __construct(private array $segments)
    {
    }

    /**
     * Starts a path with a move-to.
     */
    public static function start(float $x, float $y): self
    {
        return (new self([]))->moveTo($x, $y);
    }

    /**
     * Wraps an already encoded spec after validating it.
     *
     * @throws InvalidArgumentException when the spec is malformed
     */
    public static function fromSpec(string $spec): self
    {
        return new self([self::assert($spec)]);
    }

    /**
     * Validates a spec string and returns it unchanged.
     *
     * @throws InvalidArgumentException when the spec is malformed
     */
    public static function assert(string $spec): string
    {
        if ($spec === '' || strlen($spec) > self::MAX_SPEC_LENGTH) {
            throw new InvalidArgumentException('Canvas path specs must be 1 to 4096 characters.');
        }

        if (preg_match('/^[MLCQZ0-9 .+\-eE]+$/', $spec) !== 1) {
            throw new InvalidArgumentException('Canvas path specs accept only M, L, C, Q, Z and numbers.');
        }

        $tokens = preg_split('/\s+|(?<=[MLCQZ])|(?=[MLCQZ])/', trim($spec), -1, PREG_SPLIT_NO_EMPTY);
        $verb = null;
        $pending = 0;
        $seenMove = false;

        foreach ($tokens === false ? [] : $tokens as $token) {
            if (isset(self::ARITY[$token])) {
                if ($pending !== 0) {
                    throw new InvalidArgumentException("Canvas path verb {$verb} is missing coordinates.");
                }

                if (!$seenMove && $token !== 'M') {
                    throw new InvalidArgumentException('Canvas path specs must start with M.');
                }

                $verb = $token;
                $pending = self::ARITY[$token];
                $seenMove = true;
                continue;
            }

            if ($verb === null || $pending === 0 || !is_numeric($token) || !is_finite((float) $token)) {
                throw new InvalidArgumentException('Canvas path specs must contain finite numbers.');
            }

            $pending--;
        }

        if ($verb === null || $pending !== 0) {
            throw new InvalidArgumentException('Canvas path spec is incomplete.');
        }

        return $spec;
    }

    public function moveTo(float $x, float $y): self
    {
        return $this->append('M', $x, $y);
    }

    public function lineTo(float $x, float $y): self
    {
        return $this->append('L', $x, $y);
    }

    /**
     * Cubic Bézier to ($x, $y) with control points ($x1, $y1) and ($x2, $y2).
     */
    public function curveTo(float $x1, float $y1, float $x2, float $y2, float $x, float $y): self
    {
        return $this->append('C', $x1, $y1, $x2, $y2, $x, $y);
    }

    /**
     * Quadratic Bézier to ($x, $y) with control point ($cx, $cy).
     */
    public function quadTo(float $cx, float $cy, float $x, float $y): self
    {
        return $this->append('Q', $cx, $cy, $x, $y);
    }

    /**
     * Closes the current sub-path.
     */
    public function close(): self
    {
        return $this->append('Z');
    }

    /**
     * The encoded spec sent on the wire.
     */
    public function spec(): string
    {
        $spec = implode(' ', $this->segments);

        if (strlen($spec) > self::MAX_SPEC_LENGTH) {
            throw new InvalidArgumentException('Canvas path specs must be 1 to 4096 characters.');
        }

        return $spec;
    }

    public function __toString(): string
    {
        return $this->spec();
    }

    private function append(string $verb, float ...$numbers): self
    {
        $segment = $verb . implode(' ', array_map(Numbers::format(...), $numbers));

        return new self([...$this->segments, $segment]);
    }
}
