<?php

declare(strict_types=1);

namespace Pam\Desktop;

use InvalidArgumentException;

/**
 * A top-level menu of the application menu bar, or the menu of a tray icon.
 *
 * ```php
 * App::menu(
 *     Menu::make('File',
 *         MenuItem::make('New', $this->newDocument(...), 'CmdOrCtrl+N'),
 *         MenuItem::separator(),
 *         MenuItem::role(MenuRole::Quit),
 *     ),
 *     Menu::edit(),
 * );
 * ```
 */
final readonly class Menu
{
    /** @param list<MenuItem> $items */
    private function __construct(
        public string $label,
        public array $items,
    ) {
    }

    public static function make(string $label, MenuItem ...$items): self
    {
        if (trim($label) === '') {
            throw new InvalidArgumentException('A menu needs a label.');
        }

        return new self($label, array_values($items));
    }

    /** The standard Edit menu: undo, redo, cut, copy, paste and select all. */
    public static function edit(?string $label = null): self
    {
        return new self($label ?? \Pam\Desktop\I18n\FrameworkText::get('menu.edit'), [
            MenuItem::role(MenuRole::Undo),
            MenuItem::role(MenuRole::Redo),
            MenuItem::separator(),
            MenuItem::role(MenuRole::Cut),
            MenuItem::role(MenuRole::Copy),
            MenuItem::role(MenuRole::Paste),
            MenuItem::role(MenuRole::SelectAll),
        ]);
    }

    /** The standard Window menu: minimize, zoom and full screen. */
    public static function window(?string $label = null): self
    {
        return new self($label ?? \Pam\Desktop\I18n\FrameworkText::get('menu.window'), [
            MenuItem::role(MenuRole::Minimize),
            MenuItem::role(MenuRole::Zoom),
            MenuItem::separator(),
            MenuItem::role(MenuRole::ToggleFullScreen),
        ]);
    }
}
