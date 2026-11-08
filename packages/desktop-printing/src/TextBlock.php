<?php

declare(strict_types=1);
namespace Pam\DesktopPrinting;
use InvalidArgumentException;
final readonly class TextBlock
{
    public function __construct(public string $text, public float $x, public float $y, public float $width, public float $height, public float $size = 10, public bool $bold = false, public TextAlignment $alignment = TextAlignment::Left)
    {
        foreach ([$x, $y, $width, $height, $size] as $value) if (!is_finite($value)) throw new InvalidArgumentException('Print coordinates must be finite.');
        if ($x < 0 || $y < 0 || $width < 1 || $height < 1 || $size < 4 || $size > 144 || str_contains($text, "\0") || strlen($text) > 1_048_576) throw new InvalidArgumentException('Invalid print text block.');
    }
    /** @return array{text:string,x:float,y:float,width:float,height:float,size:float,bold:bool,align:int} */
    public function toArray(): array { return ['text' => $this->text, 'x' => $this->x, 'y' => $this->y, 'width' => $this->width, 'height' => $this->height, 'size' => $this->size, 'bold' => $this->bold, 'align' => $this->alignment->value]; }
}
