<?php

declare(strict_types=1);

namespace Pam\DesktopUi;

use Pam\Desktop\UI\WindowLayer;

/**
 * The window toast stack, like PrimeVue's ToastService: one stack for the
 * main window, drawn in the window layer (above modals and sheets) and kept
 * while the app navigates between screens or routes.
 *
 * ```php
 * Toast::service()->success('Saved', 'The product was updated.');
 * Toast::service()->error('Failed', $message, life: 4500);
 * ```
 *
 * Nothing is mounted while the stack is empty. The `<Toast>` component keeps
 * working for a stack bound to one screen or portal host.
 */
final class ToastService
{
    public const string HOST = 'pam-window-toast';
    private const string LAYER = 'desktop-ui.toast';

    private static ?self $instance = null;

    private Toast $toast;
    /** @var list<ToastMessage> the messages on screen, oldest first (same as `Toast::$messages`) */
    public array $messages {
        // By reference so `end($service->messages)` works like on a Toast.
        &get => $this->toast->messages;
    }
    private ?ToastOutlet $outlet = null;
    /** @var array{0: ToastPosition, 1: int, 2: float, 3: float, 4: string, 5: float, 6?: ToastTheme} */
    private array $geometry = [ToastPosition::TopRight, 5, 400.0, 20.0, 'system-ui', 1.5];

    private function __construct()
    {
        $this->toast = $this->create();
    }

    public static function instance(): self
    {
        return self::$instance ??= new self();
    }

    /**
     * Geometry of the stack (the `<Toast>` arguments). Replaces the stack:
     * call it once at boot, before the first message.
     */
    public function configure(
        ToastPosition $position = ToastPosition::TopRight,
        int $limit = 5,
        float $width = 400,
        float $offset = 20,
        string $fontFamily = 'system-ui',
        float $backdropBlur = 1.5,
        ToastTheme $theme = new ToastTheme(),
    ): self {
        $this->geometry = [$position, $limit, $width, $offset, $fontFamily, $backdropBlur, $theme];
        $toast = $this->create();
        $this->toast->onEmpty(null);
        $this->toast->clear();
        $this->outlet = null;
        WindowLayer::set(self::LAYER, null);
        $this->toast = $toast;

        return $this;
    }

    /** Queues a message and returns its id (for remove()). */
    public function add(ToastSeverity $severity, string $summary, string $detail = '', ?int $life = 3000, bool $closable = true): int
    {
        if ($this->outlet !== null && !WindowLayer::has(self::LAYER)) {
            // The runtime was shut down (tests, hot reload): start a fresh stack.
            $this->outlet = null;
            $this->toast->onEmpty(null);
            $this->toast = $this->create();
        }
        $id = $this->toast->add($severity, $summary, $detail, $life, $closable);
        if ($this->outlet === null) {
            $this->outlet = new ToastOutlet($this->toast);
            WindowLayer::set(self::LAYER, $this->outlet);
        }

        return $id;
    }

    public function success(string $summary, string $detail = '', ?int $life = 3000): int { return $this->add(ToastSeverity::Success, $summary, $detail, $life); }
    public function info(string $summary, string $detail = '', ?int $life = 3000): int { return $this->add(ToastSeverity::Info, $summary, $detail, $life); }
    public function warn(string $summary, string $detail = '', ?int $life = 3000): int { return $this->add(ToastSeverity::Warn, $summary, $detail, $life); }
    public function error(string $summary, string $detail = '', ?int $life = 3000): int { return $this->add(ToastSeverity::Error, $summary, $detail, $life); }

    public function remove(int $id): void
    {
        $this->toast->remove($id);
    }

    public function clear(): void
    {
        $this->toast->clear();
    }

    /** @return list<ToastMessage> the messages on screen, oldest first */
    public function messages(): array
    {
        return $this->toast->messages;
    }

    /** @internal Tests: forgets the stack. */
    public static function reset(): void
    {
        if (self::$instance === null) return;
        self::$instance->toast->onEmpty(null);
        self::$instance->toast->clear();
        WindowLayer::set(self::LAYER, null);
        self::$instance = null;
    }

    private function create(): Toast
    {
        $toast = new Toast(self::HOST, ...$this->geometry);
        $toast->onEmpty($this->hide(...));

        return $toast;
    }

    /**
     * The last message left: the layer is unmounted (nothing renders while
     * empty) and the next message starts a fresh stack component.
     */
    private function hide(): void
    {
        if ($this->outlet === null) return;
        $this->outlet = null;
        WindowLayer::set(self::LAYER, null);
        $this->toast->onEmpty(null);
        $this->toast = $this->create();
    }
}
