<?php

declare(strict_types=1);

use Pam\Desktop\App;
use Pam\Desktop\Attributes\State;
use Pam\Desktop\Component;
use Pam\Desktop\Dialog;
use Pam\Desktop\FileFilter;
use Pam\Desktop\KeyEvent;
use Pam\Desktop\Menu;
use Pam\Desktop\MenuItem;
use Pam\Desktop\MenuRole;
use Pam\Desktop\Notification;
use Pam\Desktop\Shortcut;
use Pam\Desktop\Style;
use Pam\Desktop\UI\Column;
use Pam\Desktop\UI\Pressable;
use Pam\Desktop\UI\Row;
use Pam\Desktop\UI\Text;
use Pam\Desktop\Window;
use Pam\Desktop\WindowState;

require __DIR__.'/../../vendor/autoload.php';

/**
 * Exercises the desktop shell: the application menu bar, context menus,
 * shortcuts, file dialogs, notifications, window commands, pointer cursors,
 * tooltips, hover and keyboard events.
 */
final class Shell extends Component
{
    /** @var list<string> */
    #[State]
    public array $documents = ['Roadmap.md', 'Budget 2027.xlsx', 'Release notes.md', 'Logo.png'];

    #[State]
    public int $selected = 0;

    #[State]
    public int $hovered = -1;

    #[State]
    public string $status = 'Ready';

    #[State]
    public bool $sidebar = true;

    #[State]
    public string $window = 'normal';

    public function mount(): void
    {
        $this->installMenu();
        Shortcut::register('CmdOrCtrl+K', function (): void {
            $this->status = 'Shortcut: quick search';
        });
        Shortcut::register('Delete', fn () => $this->remove($this->selected));
        Window::onStateChange(function (WindowState $state): void {
            $this->window = match (true) {
                $state->fullScreen => 'full screen',
                $state->maximized => 'maximized',
                $state->minimized => 'minimized',
                default => 'normal',
            }.sprintf(' · %d×%d', $state->width, $state->height);
        });
        Window::onFilesDrop(function (array $paths): void {
            foreach ($paths as $path) {
                $this->documents[] = basename($path);
            }
            $this->status = count($paths).' file(s) dropped';
        });
    }

    private function installMenu(): void
    {
        App::menu(
            Menu::make(
                'File',
                MenuItem::make('New document', $this->create(...), 'CmdOrCtrl+N'),
                MenuItem::make('Open…', $this->open(...), 'CmdOrCtrl+O'),
                MenuItem::make('Export…', $this->export(...), 'CmdOrCtrl+Shift+E'),
                MenuItem::separator(),
                MenuItem::role(MenuRole::Quit),
            ),
            Menu::edit(),
            Menu::make(
                'View',
                MenuItem::checkbox('Sidebar', $this->sidebar, function (bool $visible): void {
                    $this->sidebar = $visible;
                    $this->installMenu();
                }, 'CmdOrCtrl+B'),
                MenuItem::separator(),
                MenuItem::make('Maximize', Window::maximize(...)),
                MenuItem::make('Restore', Window::restore(...)),
                MenuItem::role(MenuRole::ToggleFullScreen),
            ),
            Menu::make(
                'Help',
                MenuItem::make('Notify me', function (): void {
                    Notification::show('PAM Desktop', 'Notifications come from the operating system.');
                    $this->status = 'Notification sent';
                }),
            ),
        );
    }

    private function create(): void
    {
        $this->documents[] = 'Untitled '.(count($this->documents) + 1).'.md';
        $this->selected = count($this->documents) - 1;
        $this->status = 'Document created';
    }

    private function open(): void
    {
        Dialog::openFiles(function (array $paths): void {
            foreach ($paths as $path) {
                $this->documents[] = basename($path);
            }
            $this->status = $paths === [] ? 'Open cancelled' : count($paths).' file(s) opened';
        }, 'Open documents', [new FileFilter('Documents', 'md', 'txt', 'xlsx'), FileFilter::images(), FileFilter::all()]);
    }

    private function export(): void
    {
        Dialog::saveFile(function (?string $path): void {
            $this->status = $path === null ? 'Export cancelled' : 'Exported to '.$path;
        }, $this->documents[$this->selected] ?? 'document.md', 'Export document');
    }

    private function remove(int $index): void
    {
        if (!isset($this->documents[$index])) {
            return;
        }
        $name = $this->documents[$index];
        array_splice($this->documents, $index, 1);
        $this->selected = max(0, min($this->selected, count($this->documents) - 1));
        $this->status = "Deleted {$name}";
    }

    private function key(KeyEvent $event): void
    {
        $last = count($this->documents) - 1;
        $this->selected = match ($event->key) {
            'ArrowDown' => min($last, $this->selected + 1),
            'ArrowUp' => max(0, $this->selected - 1),
            'Home' => 0,
            'End' => $last,
            default => $this->selected,
        };
        $this->status = 'Key: '.($event->control ? 'Ctrl+' : '').$event->key;
    }

    public function render(): Column
    {
        $muted = new Style(fontSize: 12, textColor: 0xFF8A93A3);

        return Column::make(
            // A custom title strip: dragging it moves the window.
            Row::make(
                Text::make('PAM Desktop shell')->style(new Style(fontSize: 13, fontWeight: 600, textColor: 0xFFE8ECF3, flexGrow: 1)),
                Text::make($this->window)->style($muted),
            )->windowDrag()->style(new Style(
                paddingLeft: 16,
                paddingRight: 16,
                height: 40,
                alignItems: Pam\Desktop\Align::Center,
                backgroundColor: 0xFF171B24,
            )),
            Row::make(
                $this->sidebar ? $this->sidebar() : null,
                $this->detail(),
            )->style(new Style(flexGrow: 1)),
            Row::make(
                Text::make($this->status)->style(new Style(fontSize: 12, textColor: 0xFFC9D1DD, flexGrow: 1)),
                Text::make('Ctrl+K search · Delete removes · right click for more')->style($muted),
            )->style(new Style(
                paddingLeft: 16,
                paddingRight: 16,
                height: 30,
                alignItems: Pam\Desktop\Align::Center,
                backgroundColor: 0xFF171B24,
            )),
        )->style(new Style(flexGrow: 1, backgroundColor: 0xFF0F1218));
    }

    private function sidebar(): Column
    {
        $rows = [];
        foreach ($this->documents as $index => $name) {
            $active = $index === $this->selected;
            $rows[] = Pressable::make(
                Text::make($name)->style(new Style(
                    fontSize: 14,
                    fontWeight: $active ? 600 : 400,
                    textColor: $active ? 0xFFFFFFFF : 0xFFC9D1DD,
                )),
            )
                ->key($name.$index)
                ->cursor('pointer')
                ->tooltip("Open {$name}")
                ->onPress(function () use ($index): void {
                    $this->selected = $index;
                })
                ->onHoverIn(function () use ($index): void {
                    $this->hovered = $index;
                })
                ->onHoverOut(function () use ($index): void {
                    if ($this->hovered === $index) {
                        $this->hovered = -1;
                    }
                })
                ->contextMenu(
                    MenuItem::make('Open', function () use ($index): void {
                        $this->selected = $index;
                    }),
                    MenuItem::make('Duplicate', function () use ($name): void {
                        $this->documents[] = 'Copy of '.$name;
                        $this->status = "Duplicated {$name}";
                    }),
                    MenuItem::separator(),
                    MenuItem::make('Delete', fn () => $this->remove($index), destructive: true),
                )
                ->style(new Style(
                    paddingLeft: 12,
                    paddingRight: 12,
                    height: 36,
                    justifyContent: Pam\Desktop\Justify::Center,
                    borderRadius: 8,
                    backgroundColor: $active ? 0xFF2F6BFF : ($index === $this->hovered ? 0xFF222838 : 0x00000000),
                ));
        }

        return Column::make(
            Text::make('DOCUMENTS')->style(new Style(fontSize: 11, fontWeight: 700, textColor: 0xFF6B7485, marginBottom: 8, marginLeft: 12)),
            ...$rows,
        )
            ->onKeyDown($this->key(...))
            ->style(new Style(width: 240, padding: 12, gap: 2, backgroundColor: 0xFF141821));
    }

    private function detail(): Column
    {
        $name = $this->documents[$this->selected] ?? 'No document';

        return Column::make(
            Text::make($name)->style(new Style(fontSize: 26, fontWeight: 700, textColor: 0xFFFFFFFF)),
            Text::make('The menu bar, shortcuts, dialogs and notifications on this screen are the native ones of the operating system.')
                ->style(new Style(fontSize: 14, lineHeight: 21, textColor: 0xFF9AA4B2)),
            Row::make(
                $this->action('Open…', 'Ctrl+O', $this->open(...)),
                $this->action('Export…', 'Ctrl+Shift+E', $this->export(...)),
                $this->action('New', 'Ctrl+N', $this->create(...)),
            )->style(new Style(gap: 10, marginTop: 8)),
        )->style(new Style(flexGrow: 1, padding: 28, gap: 12));
    }

    private function action(string $label, string $hint, Closure $handler): Pressable
    {
        return Pressable::make(
            Text::make($label)->style(new Style(fontSize: 14, fontWeight: 600, textColor: 0xFFFFFFFF)),
            Text::make($hint)->style(new Style(fontSize: 11, textColor: 0xFFB9C6FF)),
        )
            ->cursor('pointer')
            ->tooltip("{$label} ({$hint})")
            ->onPress($handler)
            ->style(new Style(
                paddingLeft: 16,
                paddingRight: 16,
                paddingTop: 10,
                paddingBottom: 10,
                borderRadius: 10,
                gap: 2,
                backgroundColor: 0xFF2F6BFF,
            ));
    }
}

App::run(new Shell());
