<?php

declare(strict_types=1);

namespace Pam\Desktop\UI;

use InvalidArgumentException;
use Pam\Desktop\Element;
use Pam\Desktop\Internal\PortalRegistry;
use Pam\Desktop\Internal\Runtime;
use Pam\Desktop\ModalPresentation;
use Pam\Desktop\PointerEvents;
use Pam\Desktop\PositionType;
use Pam\Desktop\Renderable;
use Pam\Desktop\Style;

/**
 * Content drawn above everything in the main window, modals and sheets
 * included, that survives navigation (the DesktopUI toast service): the
 * runtime mounts it next to the application root in one
 * `ModalPresentation::Overlay` modal, present only while a layer has content.
 *
 * Layer content is touch-transparent (`box-none`): only what it draws takes
 * presses. Keep the same Renderable instance between set() calls so its
 * component state is kept.
 *
 * ```php
 * WindowLayer::set('my-banner', $this->banner); // shown
 * WindowLayer::set('my-banner', null);          // removed
 * ```
 */
final class WindowLayer
{
    /** @var array<string, Renderable> */
    private static array $layers = [];

    private function __construct()
    {
    }

    public static function set(string $key, ?Renderable $content): void
    {
        if (preg_match('/^[A-Za-z0-9_.:-]{1,64}$/D', $key) !== 1) {
            throw new InvalidArgumentException('Window layer keys must be 1-64 letters, digits, "_", ".", ":" or "-".');
        }
        if ($content === null) {
            if (!isset(self::$layers[$key])) return;
            unset(self::$layers[$key]);
        } else {
            if ((self::$layers[$key] ?? null) === $content) return;
            self::$layers[$key] = $content;
        }
        Runtime::requestRender();
    }

    public static function has(string $key): bool
    {
        return isset(self::$layers[$key]);
    }

    /** @internal Runtime: the root with the layer modal appended. */
    public static function attach(Element $root): Element
    {
        $portals = PortalRegistry::instance()->has(Portal::WINDOW_HOST);
        if (self::$layers === [] && !$portals) return $root;
        $fill = new Style(
            positionType: PositionType::Absolute,
            left: 0.0,
            top: 0.0,
            right: 0.0,
            bottom: 0.0,
            pointerEvents: PointerEvents::BoxNone,
        );
        $layers = [];
        foreach (self::$layers as $key => $content) {
            $layers[] = View::make($content)->key('window-layer:'.$key)->style($fill);
        }
        if ($portals) {
            // Last: popovers and tooltips draw above the other layers.
            $layers[] = View::make(Portal::host(Portal::WINDOW_HOST))->key('window-layer:portals')->style($fill);
        }
        $modal = Modal::make(View::make(...$layers)->style($fill), true, ModalPresentation::Overlay)
            ->transparent()
            ->key('pam:window-layer')
            ->style(new Style(pointerEvents: PointerEvents::BoxNone));

        return $root->domWithChildren([...$root->children(), $modal]);
    }

    /** @internal Tests and runtime shutdown. */
    public static function reset(): void
    {
        self::$layers = [];
    }
}
