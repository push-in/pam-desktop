<?php

declare(strict_types=1);

namespace Pam\Desktop;

use Closure;
use InvalidArgumentException;
use Pam\Desktop\Internal\ShellEvents;

/**
 * The main window of the application.
 *
 * ```php
 * Window::title('Invoices — March');
 * Window::onCloseRequested(fn () => $this->confirmDiscard());
 * Window::onStateChange(fn (WindowState $state) => $this->maximized = $state->maximized);
 * ```
 */
final class Window
{
    private function __construct()
    {
    }

    public static function title(string $title): void
    {
        self::command('title', ['title' => $title]);
    }

    public static function resize(float $width, float $height): void
    {
        if ($width < 1 || $height < 1) {
            throw new InvalidArgumentException('A window needs a positive size.');
        }
        self::command('size', ['width' => $width, 'height' => $height]);
    }

    /** The smallest size the user can give the window; `null` removes the limit. */
    public static function minimumSize(?float $width, ?float $height = null): void
    {
        self::command('minSize', $width === null || $height === null ? [] : ['width' => $width, 'height' => $height]);
    }

    public static function resizable(bool $enabled = true): void
    {
        self::command('resizable', ['enabled' => $enabled]);
    }

    public static function minimize(): void
    {
        self::command('minimize');
    }

    public static function maximize(): void
    {
        self::command('maximize');
    }

    /** Leaves the maximized, minimized or full screen state. */
    public static function restore(): void
    {
        self::command('restore');
    }

    public static function fullScreen(bool $enabled = true): void
    {
        self::command('fullScreen', ['enabled' => $enabled]);
    }

    /** Brings the window to the front and gives it keyboard focus. */
    public static function focus(): void
    {
        self::command('focus');
    }

    public static function center(): void
    {
        self::command('center');
    }

    public static function alwaysOnTop(bool $enabled = true): void
    {
        self::command('alwaysOnTop', ['enabled' => $enabled]);
    }

    public static function show(): void
    {
        self::command('show');
    }

    /** Hides the window without quitting; pair it with a tray icon. */
    public static function hide(): void
    {
        self::command('hide');
    }

    /**
     * Progress shown on the taskbar or dock icon, from 0 to 1; `null`
     * removes it.
     */
    public static function progress(?float $value): void
    {
        self::command('progress', $value === null ? [] : ['value' => max(0.0, min(1.0, $value))]);
    }

    /** Closes the window and ends the application, without asking. */
    public static function close(): void
    {
        self::command('close');
    }

    /**
     * Lets the application decide what happens when the user closes the
     * window (unsaved changes, hide to tray…). Call `Window::close()` from
     * the handler to really close. Pass `null` to restore the default.
     */
    public static function onCloseRequested(?Closure $handler): void
    {
        if ($handler === null) {
            ShellEvents::forget('window', 'close-requested');
            self::command('interceptClose', ['enabled' => false]);

            return;
        }
        ShellEvents::handle('window', 'close-requested', static fn (): mixed => $handler());
        self::command('interceptClose', ['enabled' => true]);
    }

    /**
     * Runs `$listener` when the window is maximized, minimized, focused,
     * resized or enters full screen. Returns a subscription for `offStateChange`.
     *
     * @param Closure(WindowState): mixed $listener
     */
    public static function onStateChange(Closure $listener): int
    {
        return ShellEvents::subscribe('window', static function (string $id, array $data) use ($listener): void {
            if ($id === 'state') {
                $listener(WindowState::fromArray($data));
            }
        });
    }

    public static function offStateChange(int $subscription): void
    {
        ShellEvents::unsubscribe('window', $subscription);
    }

    /** @param Closure(WindowState): mixed $completed */
    public static function state(Closure $completed): void
    {
        ShellEvents::send(
            'windowState',
            [],
            static function (array $values) use ($completed): void {
                $completed(WindowState::fromJson(is_string($values['state'] ?? null) ? $values['state'] : '{}'));
            },
            static function () use ($completed): void {
                $completed(new WindowState());
            },
        );
    }

    /**
     * Files dropped on the window outside any element with `onFilesDrop`.
     *
     * @param Closure(list<string>): mixed $listener
     */
    public static function onFilesDrop(Closure $listener): int
    {
        return ShellEvents::subscribe('drop', static function (string $id, array $data) use ($listener): void {
            $paths = array_values(array_filter(is_array($data['paths'] ?? null) ? $data['paths'] : [], is_string(...)));
            if ($paths !== []) {
                $listener($paths);
            }
        });
    }

    public static function offFilesDrop(int $subscription): void
    {
        ShellEvents::unsubscribe('drop', $subscription);
    }

    /** @param array<string, bool|float|int|string> $payload */
    private static function command(string $command, array $payload = []): void
    {
        ShellEvents::send('window', ['command' => $command] + $payload);
    }
}
