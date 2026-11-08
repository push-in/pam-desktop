<?php

declare(strict_types=1);

namespace Pam\Desktop;

use Closure;
use InvalidArgumentException;
use Pam\Desktop\Internal\ShellEvents;

/**
 * Desktop notifications shown by the operating system.
 *
 * ```php
 * Notification::show('Export finished', 'report.pdf is ready', fn () => $this->reveal());
 * ```
 */
final class Notification
{
    private function __construct()
    {
    }

    /**
     * @param Closure|null $onActivate the user clicked the notification
     * @return string the id of the notification
     */
    public static function show(string $title, string $body = '', ?Closure $onActivate = null): string
    {
        if (trim($title) === '') {
            throw new InvalidArgumentException('A notification needs a title.');
        }
        $id = ShellEvents::id('n');
        if ($onActivate !== null) {
            ShellEvents::handle('notification', $id, static function () use ($onActivate, $id): void {
                ShellEvents::forget('notification', $id);
                $onActivate();
            });
        }
        ShellEvents::send('notify', ['id' => $id, 'title' => $title, 'body' => $body]);

        return $id;
    }
}
