<?php

declare(strict_types=1);
namespace Pam\DesktopPrinting;
use InvalidArgumentException;

/** Monospaced receipt layout; Unicode display widths wrap without truncation. */
final class Receipt
{
    /** @var list<array{text:string,size:float,bold:bool,align:TextAlignment}> */
    private array $lines = [];
    public function __construct(private readonly string $title, private readonly float $width = 226.77, private readonly float $margin = 12)
    {
        if (!is_finite($width) || !is_finite($margin) || $width < 72 || $width > 2880 || $margin < 0 || $margin * 2 + 20 > $width) throw new InvalidArgumentException('Invalid receipt width or margin.');
    }
    public function text(string $text, bool $bold = false, float $size = 10, TextAlignment $alignment = TextAlignment::Left): self
    {
        if (!is_finite($size) || $size < 4 || $size > 144 || strlen($text) > 1_048_576 || !mb_check_encoding($text, 'UTF-8')) throw new InvalidArgumentException('Invalid receipt text.');
        $columns = max(1, (int) floor(($this->width - $this->margin * 2) / ($size * 0.65)));
        foreach (explode("\n", str_replace(["\r\n", "\r"], "\n", $text)) as $line) {
            do {
                $fragment = mb_strimwidth($line, 0, $columns, '', 'UTF-8');
                if ($fragment === '' && $line !== '') throw new InvalidArgumentException('Receipt font is too large for the paper.');
                $this->lines[] = ['text' => $fragment, 'size' => $size, 'bold' => $bold, 'align' => $alignment];
                if (count($this->lines) > 5000) throw new InvalidArgumentException('Receipt exceeds 5,000 lines.');
                $line = mb_substr($line, mb_strlen($fragment, 'UTF-8'), null, 'UTF-8');
            } while ($line !== '');
        }
        return $this;
    }
    public function pair(string $left, string $right, bool $bold = false): self
    {
        $columns = (int) floor(($this->width - $this->margin * 2) / 6.5);
        $space = $columns - mb_strwidth($left, 'UTF-8') - mb_strwidth($right, 'UTF-8');
        if ($space > 0 && !str_contains($left.$right, "\n")) return $this->text($left.str_repeat(' ', $space).$right, $bold);
        return $this->text($left, $bold)->text($right, $bold, alignment: TextAlignment::Right);
    }
    public function separator(): self { return $this->text(str_repeat('-', max(1, (int) floor(($this->width - $this->margin * 2) / 6.5)))); }
    public function document(?float $pageHeight = null): Document
    {
        if ($pageHeight !== null && (!is_finite($pageHeight) || $pageHeight < max(72, $this->margin * 2 + 216) || $pageHeight > 8640)) throw new InvalidArgumentException('Invalid fixed paper height.');
        $pages = []; $page = []; $y = $this->margin; $maxY = $y;
        foreach ($this->lines as $line) {
            $height = $line['size'] * 1.5;
            if ($y + $height > ($pageHeight ?? 8500) - $this->margin && $page !== []) { $pages[] = $page; $page = []; $maxY = max($maxY, $y); $y = $this->margin; }
            $page[] = new TextBlock($line['text'], $this->margin, $y, $this->width - $this->margin * 2, $height, $line['size'], $line['bold'], $line['align']);
            $y += $height;
        }
        if ($page !== [] || $pages === []) $pages[] = $page;
        return new Document($this->title, $this->width, ($pageHeight ?? max(72, max($maxY, $y) + $this->margin)), $pages);
    }
}
