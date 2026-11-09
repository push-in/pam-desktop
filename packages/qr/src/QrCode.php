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
use Pam\Desktop\ImageFit;
use Pam\Desktop\UI\Image;
use Pam\Desktop\PropKey;
use Pam\Desktop\Renderable;

/** Retained, locally encoded QR. No images, network, native extension or background work. */
final readonly class QrCode implements Renderable
{
    public const int MAX_CONTENT_BYTES = 7089;
    public const int QUIET_MODULES = 4;
    public CanvasScene $scene;
    /** The bitmap of `image` mode, null otherwise. */
    public ?string $png;
    public int $moduleCount;
    public int $moduleSize;

    /**
     * @param int $margin quiet zone in modules (the QR standard asks for 4)
     * @param bool $stretch modules of fractional size fill `size` exactly, like a QR image scaled into its box (web QR services); otherwise whole-pixel modules are centered
     * @param int|null $image draw the code as a bitmap of this many pixels (whole modules picked per pixel, like a QR image service) that the image filter scales to `size`, as a browser shows such an image
     */
    public function __construct(string $data, public int $size = 256, ErrorCorrection $correction = ErrorCorrection::Medium, public int $margin = self::QUIET_MODULES, public bool $stretch = false, public ?int $image = null)
    {
        if ($image !== null && ($image < 21 || $image > 4096)) throw new InvalidArgumentException('The QR image must be 21–4096 pixels.');
        if ($data === '' || strlen($data) > self::MAX_CONTENT_BYTES || $size < 29 || $size > 4096) throw new InvalidArgumentException('QR content must contain 1–7089 bytes and size must be 29–4096 pixels.');
        if ($margin < 0 || $margin > 16) throw new InvalidArgumentException('The QR margin must be 0–16 modules.');
        if (!class_exists(Encoder::class)) {
            throw new \LogicException('QR codes need the bacon/bacon-qr-code package: run `composer require bacon/bacon-qr-code`.');
        }
        $matrix = Encoder::encode($data, $correction->encoder(), 'UTF-8')->getMatrix();
        $this->moduleCount = $matrix->getWidth();
        $this->moduleSize = intdiv($size, $this->moduleCount + $margin * 2);
        if ($this->moduleSize < 1) throw new InvalidArgumentException('The QR size is too small for its encoded data and quiet zone.');
        $module = $stretch ? $size / ($this->moduleCount + $margin * 2) : (float) $this->moduleSize;
        $origin = $stretch ? $margin * $module : (float) intdiv($size - $this->moduleCount * $this->moduleSize, 2);
        $at = static fn (int $index): string => rtrim(rtrim(sprintf('%.4F', $origin + $index * $module), '0'), '.');
        $canvas = (new Canvas())->fillRect(0, 0, $size, $size, '#FFFFFF');
        $path = '';
        for ($row = 0; $row < $this->moduleCount; ++$row) {
            for ($column = 0; $column < $this->moduleCount;) {
                if ($matrix->get($column, $row) !== 1) { ++$column; continue; }
                $start = $column;
                while ($column < $this->moduleCount && $matrix->get($column, $row) === 1) ++$column;
                $left = $at($start);
                $right = $at($column);
                $top = $at($row);
                $bottom = $at($row + 1);
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
        $this->png = $image === null ? null : self::png($matrix, $this->moduleCount, $margin, $image);
    }

    /** Grayscale PNG of `pixels`², each pixel taking the module under it (nearest, no smoothing). */
    private static function png(object $matrix, int $count, int $margin, int $pixels): string
    {
        $total = $count + 2 * $margin;
        $raw = '';
        for ($y = 0; $y < $pixels; ++$y) {
            $row = intdiv($y * $total, $pixels) - $margin;
            $line = "\0";
            for ($x = 0; $x < $pixels; ++$x) {
                $column = intdiv($x * $total, $pixels) - $margin;
                $dark = $row >= 0 && $row < $count && $column >= 0 && $column < $count && $matrix->get($column, $row) === 1;
                $line .= $dark ? "\0" : "\xFF";
            }
            $raw .= $line;
        }
        $chunk = static fn (string $type, string $body): string => pack('N', strlen($body)).$type.$body.pack('N', crc32($type.$body));

        return "\x89PNG\r\n\x1A\n".$chunk('IHDR', pack('NNCCCCC', $pixels, $pixels, 8, 0, 0, 0, 0)).$chunk('IDAT', (string) gzcompress($raw, 6)).$chunk('IEND', '');
    }

    public static function make(string $data, int $size = 256, ErrorCorrection $correction = ErrorCorrection::Medium, int $margin = self::QUIET_MODULES, bool $stretch = false, ?int $image = null): self { return new self($data, $size, $correction, $margin, $stretch, $image); }
    public function toElement(): Element
    {
        if ($this->png !== null) {
            return Image::make('data:image/png;base64,'.base64_encode($this->png))->fit(ImageFit::Fill)->fadeDuration(0)->toElement()->property(PropKey::Width, $this->size)->property(PropKey::Height, $this->size);
        }
        return CanvasView::make($this->scene)->toElement()->property(PropKey::Width, $this->size)->property(PropKey::Height, $this->size);
    }
}
