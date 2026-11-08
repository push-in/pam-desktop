<?php

declare(strict_types=1);
namespace Pam\Desktop\Qr;
use BaconQrCode\Encoder\Encoder;
use InvalidArgumentException;
use Pam\Desktop\Canvas\Canvas;
use Pam\Desktop\Canvas\CanvasScene;
use Pam\Desktop\Canvas\CanvasView;
use Pam\Desktop\Canvas\Path;
use Pam\Desktop\Element;
use Pam\Desktop\PropKey;
use Pam\Desktop\Renderable;

/** Retained, locally encoded QR. No images, network, native extension or background work. */
final readonly class QrCode implements Renderable
{
    public const int MAX_CONTENT_BYTES = 7089;
    public const int QUIET_MODULES = 4;
    public CanvasScene $scene;
    public int $moduleCount;
    public int $moduleSize;

    public function __construct(string $data, public int $size = 256, ErrorCorrection $correction = ErrorCorrection::Medium)
    {
        if ($data === '' || strlen($data) > self::MAX_CONTENT_BYTES || $size < 29 || $size > 4096) throw new InvalidArgumentException('QR content must contain 1–7089 bytes and size must be 29–4096 pixels.');
        if (!class_exists(Encoder::class)) {
            throw new \LogicException('QR codes need the bacon/bacon-qr-code package: run `composer require bacon/bacon-qr-code`.');
        }
        $matrix = Encoder::encode($data, $correction->encoder(), 'UTF-8')->getMatrix();
        $this->moduleCount = $matrix->getWidth();
        $this->moduleSize = intdiv($size, $this->moduleCount + self::QUIET_MODULES * 2);
        if ($this->moduleSize < 1) throw new InvalidArgumentException('The QR size is too small for its encoded data and four-module quiet zone.');
        $origin = intdiv($size - $this->moduleCount * $this->moduleSize, 2);
        $canvas = (new Canvas())->fillRect(0, 0, $size, $size, '#FFFFFF');
        $path = '';
        for ($row = 0; $row < $this->moduleCount; ++$row) {
            for ($column = 0; $column < $this->moduleCount;) {
                if ($matrix->get($column, $row) !== 1) { ++$column; continue; }
                $start = $column;
                while ($column < $this->moduleCount && $matrix->get($column, $row) === 1) ++$column;
                $left = $origin + $start * $this->moduleSize;
                $right = $origin + $column * $this->moduleSize;
                $top = $origin + $row * $this->moduleSize;
                $bottom = $top + $this->moduleSize;
                $rectangle = "M{$left} {$top} L{$right} {$top} L{$right} {$bottom} L{$left} {$bottom} Z ";
                if (strlen($path) + strlen($rectangle) > 3500) {
                    $canvas = $canvas->path(Path::fromSpec(trim($path)), '#000000');
                    $path = '';
                }
                $path .= $rectangle;
            }
        }
        if ($path !== '') $canvas = $canvas->path(Path::fromSpec(trim($path)), '#000000');
        $this->scene = $canvas->scene();
    }

    public static function make(string $data, int $size = 256, ErrorCorrection $correction = ErrorCorrection::Medium): self { return new self($data, $size, $correction); }
    public function toElement(): Element
    {
        return CanvasView::make($this->scene)->toElement()->property(PropKey::Width, $this->size)->property(PropKey::Height, $this->size);
    }
}
