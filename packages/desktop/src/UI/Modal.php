<?php

declare(strict_types=1);

namespace Pam\Desktop\UI;

use Closure;
use Pam\Desktop\Element;
use Pam\Desktop\EventKind;
use Pam\Desktop\ModalAnimationType;
use Pam\Desktop\ModalOrientation;
use Pam\Desktop\ModalPresentation;
use Pam\Desktop\NodeKind;
use Pam\Desktop\PropKey;
use Pam\Desktop\Renderable;

final class Modal extends Element
{
    public static function make(
        Renderable $content,
        bool $visible = true,
        ModalPresentation $presentation = ModalPresentation::Dialog,
    ): self {
        return (new self(NodeKind::Modal))
            ->withChildren([$content])
            ->withProperty(PropKey::Visible, $visible)
            ->withProperty(PropKey::ModalPresentation, $presentation->value);
    }

    public function animationType(ModalAnimationType $type): self
    {
        return $this->withProperty(PropKey::ModalAnimationType, $type->value);
    }

    public function backdropColor(int $color): self
    {
        return $this->withProperty(PropKey::ModalBackdropColor, $color);
    }

    public function transparent(bool $transparent = true): self
    {
        return $this->withProperty(PropKey::ModalTransparent, $transparent);
    }

    public function hardwareAccelerated(bool $accelerated = true): self
    {
        return $this->withProperty(
            PropKey::ModalHardwareAccelerated,
            $accelerated,
        );
    }

    public function navigationBarTranslucent(bool $translucent = true): self
    {
        return $this->withProperty(
            PropKey::ModalNavigationBarTranslucent,
            $translucent,
        );
    }

    /**
     * Android: paints the modal window's navigation bar with this colour
     * (no system scrim); its buttons turn dark on a light colour and light
     * on a dark one. Unset, the bar follows the theme and translucency.
     */
    public function navigationBarColor(int $color): self
    {
        return $this->withProperty(PropKey::ModalNavigationBarColor, $color);
    }

    public function statusBarTranslucent(bool $translucent = true): self
    {
        return $this->withProperty(
            PropKey::ModalStatusBarTranslucent,
            $translucent,
        );
    }

    public function allowSwipeDismissal(bool $allowed = true): self
    {
        return $this->withProperty(
            PropKey::ModalAllowSwipeDismissal,
            $allowed,
        );
    }

    /** @param Closure(): void $handler */
    public function onRequestClose(Closure $handler): self
    {
        return $this->withVoidEvent(EventKind::ModalRequestClose, $handler);
    }

    /** @param Closure(): void $handler */
    public function onShow(Closure $handler): self
    {
        return $this->withVoidEvent(EventKind::ModalShow, $handler);
    }

    /** @param Closure(): void $handler */
    public function onDismiss(Closure $handler): self
    {
        return $this->withVoidEvent(EventKind::ModalDismiss, $handler);
    }

    /** @param Closure(ModalOrientation): void $handler */
    public function onOrientationChange(Closure $handler): self
    {
        return $this->withEvent(
            EventKind::ModalOrientationChange,
            static function (string $payload) use ($handler): void {
                $handler(ModalOrientation::fromPayload($payload));
            },
        );
    }

    private function withVoidEvent(EventKind $kind, Closure $handler): self
    {
        return $this->withEvent(
            $kind,
            static function (string $_payload) use ($handler): void {
                $handler();
            },
        );
    }
}
