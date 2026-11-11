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
 *
 * Notification::show('Ana', 'Can you review the deck?', fn () => $this->open($thread), [
 *     'Reply' => fn () => $this->reply($thread),
 *     'Mark as read' => fn () => $this->markRead($thread),
 * ]);
 * ```
 */
final class Notification
{
    private function __construct()
    {
    }

    /**
     * @param Closure|null $onActivate the user clicked the notification
     * @param array<string, Closure> $actions buttons (at most three): label => handler
     * @return string the id of the notification
     */
    public static function show(string $title, string $body = '', ?Closure $onActivate = null, array $actions = []): string
    {
        if (trim($title) === '') {
            throw new InvalidArgumentException('A notification needs a title.');
        }
        if (count($actions) > 3) {
            throw new InvalidArgumentException('A notification shows at most three buttons.');
        }
        $buttons = [];
        $handlers = [];
        foreach ($actions as $label => $handler) {
            if (!is_string($label) || trim($label) === '' || !$handler instanceof Closure) {
                throw new InvalidArgumentException('Notification actions are label => Closure pairs.');
            }
            $key = 'a'.count($buttons);
            $buttons[] = ['id' => $key, 'label' => $label];
            $handlers[$key] = $handler;
        }
        $id = ShellEvents::id('n');
        if ($onActivate !== null || $handlers !== []) {
            ShellEvents::handle('notification', $id, static function (array $data) use ($onActivate, $handlers, $id): void {
                ShellEvents::forget('notification', $id);
                $action = $data['action'] ?? null;
                $handler = is_string($action) ? ($handlers[$action] ?? null) : $onActivate;
                $handler?->__invoke();
            });
        }
        ShellEvents::send('notify', [
            'id' => $id,
            'title' => $title,
            'body' => $body,
            'actions' => json_encode($buttons, JSON_UNESCAPED_UNICODE | JSON_THROW_ON_ERROR),
        ]);

        return $id;
    }
}
