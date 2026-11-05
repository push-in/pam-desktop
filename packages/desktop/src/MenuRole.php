<?php

declare(strict_types=1);

namespace Pam\Desktop;

/**
 * Menu items the operating system implements itself: they get the native
 * label, shortcut and behaviour of each platform.
 */
enum MenuRole: string
{
    case Quit = 'quit';
    case About = 'about';
    case Preferences = 'preferences';
    case Undo = 'undo';
    case Redo = 'redo';
    case Cut = 'cut';
    case Copy = 'copy';
    case Paste = 'paste';
    case SelectAll = 'selectAll';
    case Minimize = 'minimize';
    case Zoom = 'zoom';
    case ToggleFullScreen = 'toggleFullScreen';
    case CloseWindow = 'closeWindow';
    case Hide = 'hide';
    case HideOthers = 'hideOthers';
    case ShowAll = 'showAll';

    /** The label used when the application gives none. */
    public function label(): string
    {
        return match ($this) {
            self::Quit => 'Quit',
            self::About => 'About',
            self::Preferences => 'Preferences…',
            self::Undo => 'Undo',
            self::Redo => 'Redo',
            self::Cut => 'Cut',
            self::Copy => 'Copy',
            self::Paste => 'Paste',
            self::SelectAll => 'Select All',
            self::Minimize => 'Minimize',
            self::Zoom => 'Zoom',
            self::ToggleFullScreen => 'Toggle Full Screen',
            self::CloseWindow => 'Close Window',
            self::Hide => 'Hide',
            self::HideOthers => 'Hide Others',
            self::ShowAll => 'Show All',
        };
    }

    /** The conventional shortcut of the role, portable across systems. */
    public function shortcut(): ?string
    {
        return match ($this) {
            self::Quit => 'CmdOrCtrl+Q',
            self::Preferences => 'CmdOrCtrl+,',
            self::Undo => 'CmdOrCtrl+Z',
            self::Redo => Platform::isMacOS() ? 'Cmd+Shift+Z' : 'Ctrl+Y',
            self::Cut => 'CmdOrCtrl+X',
            self::Copy => 'CmdOrCtrl+C',
            self::Paste => 'CmdOrCtrl+V',
            self::SelectAll => 'CmdOrCtrl+A',
            self::Minimize => Platform::isMacOS() ? 'Cmd+M' : null,
            self::ToggleFullScreen => Platform::isMacOS() ? 'Ctrl+Cmd+F' : 'F11',
            self::CloseWindow => 'CmdOrCtrl+W',
            self::Hide => Platform::isMacOS() ? 'Cmd+H' : null,
            default => null,
        };
    }
}
