<?php

declare(strict_types=1);
namespace Pam\DesktopPrinting;
use InvalidArgumentException;

/** Paper size and positions are points (1/72 inch); native systems draw Unicode text. */
final readonly class Document
{
    /** @var list<list<TextBlock>> */
    public array $pages;
    /** @param array<array-key, mixed> $pages Raw caller input is validated into bounded typed pages. */
    public function __construct(public string $title, public float $width, public float $height, array $pages)
    {
        if ($title === '' || strlen($title) > 256 || str_contains($title, "\0") || !is_finite($width) || !is_finite($height) || $width < 72 || $width > 2880 || $height < 72 || $height > 8640 || !array_is_list($pages) || $pages === [] || count($pages) > 100) throw new InvalidArgumentException('Invalid print document or paper size.');
        $count = $bytes = 0; $validated = [];
        foreach ($pages as $page) {
            if (!is_array($page) || !array_is_list($page)) throw new InvalidArgumentException('Print pages must contain ordered text blocks.');
            $blocks = [];
            foreach ($page as $block) {
                if (!$block instanceof TextBlock || $block->x + $block->width > $width + 0.001 || $block->y + $block->height > $height + 0.001) throw new InvalidArgumentException('A print block exceeds its page.');
                ++$count; $bytes += strlen($block->text); $blocks[] = $block;
            }
            $validated[] = $blocks;
        }
        $this->pages = $validated;
        if ($count > 5000 || $bytes > 1_048_576) throw new InvalidArgumentException('Print document exceeds its bounded work budget.');
    }
    public static function fromJson(string $json): self
    {
        if (strlen($json) > 2_097_152) throw new InvalidArgumentException('Print document exceeds two megabytes.');
        $data = json_decode($json, true, 32, JSON_THROW_ON_ERROR);
        if (!is_array($data) || !is_string($data['title'] ?? null) || !is_array($data['pages'] ?? null) || !array_is_list($data['pages']) || count($data['pages']) > 100) throw new InvalidArgumentException('Invalid stored print document.');
        $number = static function (mixed $value): float { if (!is_int($value) && !is_float($value)) throw new InvalidArgumentException('Invalid print coordinate.'); return (float) $value; };
        $pages = []; $count = 0;
        foreach ($data['pages'] as $page) {
            if (!is_array($page) || !array_is_list($page)) throw new InvalidArgumentException('Invalid stored print page.');
            $blocks = [];
            foreach ($page as $block) {
                if (++$count > 5000 || !is_array($block) || !is_string($block['text'] ?? null) || !is_bool($block['bold'] ?? null) || !is_int($block['align'] ?? null)) throw new InvalidArgumentException('Invalid stored print block.');
                $blocks[] = new TextBlock($block['text'], $number($block['x'] ?? null), $number($block['y'] ?? null), $number($block['width'] ?? null), $number($block['height'] ?? null), $number($block['size'] ?? null), $block['bold'], TextAlignment::from($block['align']));
            }
            $pages[] = $blocks;
        }
        return new self($data['title'], $number($data['width'] ?? null), $number($data['height'] ?? null), $pages);
    }

    public function json(): string
    {
        $pages = [];
        foreach ($this->pages as $page) $pages[] = array_map(static fn (TextBlock $block): array => $block->toArray(), $page);
        $json = json_encode(['title' => $this->title, 'width' => $this->width, 'height' => $this->height, 'pages' => $pages], JSON_THROW_ON_ERROR | JSON_UNESCAPED_UNICODE);
        if (strlen($json) > 2_097_152) throw new InvalidArgumentException('Print document exceeds two megabytes.');
        return $json;
    }
}
