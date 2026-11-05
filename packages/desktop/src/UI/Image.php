<?php

declare(strict_types=1);

namespace Pam\Desktop\UI;

use Closure;
use InvalidArgumentException;
use Pam\Desktop\Element;
use Pam\Desktop\ImagePrefetchResult;
use Pam\Desktop\Internal\Wire;
use Pam\Desktop\MediaPriority;
use Pam\Desktop\ModuleResultStatus;
use Pam\Desktop\Modules\NativeModules;
use Pam\Desktop\NodeKind;
use Pam\Desktop\PropKey;
use Pam\Desktop\UI\Concerns\HasImageBehavior;

use function count;
use function in_array;
use function is_string;
use function strlen;

final class Image extends Element
{
    use HasImageBehavior;

    public static function make(string $source): self
    {
        return (new self(NodeKind::Image))->withProperty(PropKey::Source, $source);
    }

    /**
     * Downloads remote images into the shared image disk cache ahead of
     * rendering. Pass the same headers/cache keys the Image will use.
     *
     * Packaged `asset://` images (a screen background) are decoded once per
     * process at full resolution and kept in memory instead, so an Image with
     * that exact source paints in the frame that binds it, with no decode and
     * no fade; a theme switch between two preloaded sources only swaps them.
     * Bounded natively (8 images, 48 MiB); prefetch them while a previous
     * screen shows, not for images that may never render.
     *
     * @param list<string> $urls
     * @param array<string, string> $headers
     * @param array<string, string> $cacheKeys Optional url => stable cache key map.
     * @param null|Closure(ImagePrefetchResult): void $callback
     */
    public static function prefetch(
        array $urls,
        MediaPriority $priority = MediaPriority::Prefetch,
        array $headers = [],
        ?Closure $callback = null,
        array $cacheKeys = [],
    ): int {
        $urls = array_values(array_unique(array_filter(
            $urls,
            static fn (mixed $url): bool => is_string($url) && $url !== '',
        )));
        if ($urls === [] || count($urls) > 100) {
            throw new InvalidArgumentException('Prefetch between 1 and 100 image URLs.');
        }
        foreach ($urls as $url) {
            $scheme = strtolower((string) parse_url($url, PHP_URL_SCHEME));
            if (strlen($url) > 8_192 || !in_array($scheme, ['https', 'http', 'asset'], true)) {
                throw new InvalidArgumentException('Only remote HTTP(S) or packaged asset:// images can be prefetched.');
            }
        }
        $packedHeaders = (new self(NodeKind::Image))->headers($headers)->properties()[PropKey::ImageRequestHeaders->value] ?? '';
        $keys = array_map(static fn (string $url): string => (string) ($cacheKeys[$url] ?? ''), $urls);

        return NativeModules::call(
            'image',
            'prefetch',
            [
                'urls' => json_encode($urls, JSON_THROW_ON_ERROR | JSON_UNESCAPED_SLASHES),
                'priority' => $priority->value,
                'headers' => (string) $packedHeaders,
                'cacheKeys' => json_encode($keys, JSON_THROW_ON_ERROR | JSON_UNESCAPED_SLASHES),
            ],
            static function ($result) use ($callback, $urls): void {
                if ($result->status === ModuleResultStatus::Failure) {
                    $callback?->__invoke(new ImagePrefetchResult(0, count($urls), 0));

                    return;
                }
                $values = Wire::decodeMap($result->payload);
                $callback?->__invoke(new ImagePrefetchResult(
                    succeeded: max(0, (int) ($values['succeeded'] ?? 0)),
                    failed: max(0, (int) ($values['failed'] ?? 0)),
                    bytes: max(0, (int) ($values['bytes'] ?? 0)),
                ));
            },
        );
    }
}
