<?php

declare(strict_types=1);

use Pam\Desktop\App;
use Pam\Desktop\Attributes\State;
use Pam\Desktop\Component;
use Pam\Desktop\Navigation\NavigationPresentation;
use Pam\Desktop\Navigation\NavigationTransition;
use Pam\Desktop\Navigation\Navigator;
use Pam\Desktop\Navigation\ScreenOptions;
use Pam\Desktop\Renderable;
use Pam\Desktop\Style;
use Pam\Desktop\UI\Button;
use Pam\Desktop\UI\Column;
use Pam\Desktop\UI\Image;
use Pam\Desktop\UI\Pressable;
use Pam\Desktop\UI\Screen;
use Pam\Desktop\UI\Text;

require __DIR__.'/../../vendor/autoload.php';

/**
 * Exercises navigation chrome: the header the host draws (title, back
 * button, search) and a route presented as a form sheet.
 */
final class Stack extends Component
{
    private Navigator $navigator;

    #[State]
    public string $query = '';

    public function mount(): void
    {
        $this->navigator = new Navigator(
            initialRoute: 'inbox',
            routes: [
                'inbox' => fn (): Renderable => $this->page('Your inbox is empty.', [
                    Button::make('Open a message')->onPress(fn () => $this->navigator->push('message')),
                    Button::make('Search')->onPress(fn () => $this->navigator->push('search')),
                    Button::make('Filters (form sheet)')->onPress(fn () => $this->navigator->push('filters')),
                    Pressable::make(
                        Image::make('asset://photo.jpg')->sharedTransition('photo')->style(new Style(width: 96, height: 96, borderRadius: 16)),
                    )->onPress(fn () => $this->navigator->push('photo')),
                ]),
                'photo' => fn (): Renderable => Screen::make(Column::make(
                    Image::make('asset://photo.jpg')->sharedTransition('photo')->style(new Style(height: 320, borderRadius: 0)),
                    Text::make('The picture flew here from its thumbnail, and flies back on the way out.')
                        ->style(new Style(fontSize: 15, lineHeight: 22, textColor: 0xFF2A3140, padding: 20)),
                )->style(new Style(flexGrow: 1, paddingTop: 56, backgroundColor: 0xFFFFFFFF))),
                'message' => fn (): Renderable => $this->page('The header shows a back button because there is a route below.', []),
                'search' => fn (): Renderable => $this->page('Type in the header: the host reports every change to onHeaderSearchChange.', []),
                'filters' => fn (): Renderable => Screen::make(Column::make(
                    Text::make('Filters')->style(new Style(fontSize: 20, fontWeight: 700, textColor: 0xFF12161C)),
                    Text::make('A form sheet covers part of the route below, which stays visible.')->style(new Style(fontSize: 14, lineHeight: 21, textColor: 0xFF5B6472)),
                    Button::make('Done')->onPress(fn () => $this->navigator->pop()),
                )->style(new Style(flexGrow: 1, padding: 24, gap: 12, backgroundColor: 0xFFFFFFFF))),
            ],
            restorePersistedState: false,
            screenOptions: [
                'inbox' => new ScreenOptions(title: 'Inbox', headerShown: true),
                'message' => new ScreenOptions(title: 'Invoice 204', headerShown: true, headerBackgroundColor: 0xFF2F6BFF, headerTintColor: 0xFFFFFFFF),
                'search' => new ScreenOptions(title: 'Search', headerShown: true, headerSearchEnabled: true, headerSearchPlaceholder: 'Search messages', onHeaderSearchChange: function (string $query): void {
                    $this->query = $query;
                }),
                'photo' => new ScreenOptions(title: 'Photo', headerShown: true, animationDurationMs: 420),
                'filters' => new ScreenOptions(presentation: NavigationPresentation::FormSheet, animation: NavigationTransition::SlideFromBottom, sheetAllowedDetents: [0.45, 1.0]),
            ],
        );
    }

    /** @param list<Renderable> $actions */
    private function page(string $text, array $actions): Renderable
    {
        return Screen::make(Column::make(
            Text::make($text)->style(new Style(fontSize: 15, lineHeight: 22, textColor: 0xFF2A3140)),
            ...$actions,
        )->style(new Style(flexGrow: 1, paddingTop: 72, paddingLeft: 20, paddingRight: 20, gap: 12, backgroundColor: 0xFFF7F8FA)));
    }

    public function render(): Renderable
    {
        return $this->navigator->render();
    }
}

App::run(new Stack());
