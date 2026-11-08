<?php

declare(strict_types=1);

use Pam\Desktop\App;
use Pam\Desktop\Component;
use Pam\Desktop\Style;
use Pam\Desktop\System\Accessibility;
use Pam\Desktop\System\Linking;
use Pam\Desktop\UI\Button;
use Pam\Desktop\UI\Column;
use Pam\Desktop\UI\Text;

require __DIR__.'/../../vendor/autoload.php';

/**
 * Deep links: the application claims the `pamlinks` scheme, runs as a single
 * instance and receives the link of every later launch.
 */
final class Links extends Component
{
    public string $initial = 'asking…';

    /** @var list<string> */
    public array $received = [];

    public string $reader = 'unknown';

    public function mount(): void
    {
        Linking::initial(fn (?string $url) => $this->initial = $url ?? 'none');
        Linking::listen(fn (string $url) => $this->received[] = $url);
        Accessibility::screenReaderEnabled(fn (bool $enabled) => $this->reader = $enabled ? 'running' : 'not running');
    }

    public function render(): Column
    {
        $label = new Style(fontSize: 13, textColor: 0xFF5B6470);
        $value = new Style(fontSize: 15, textColor: 0xFF12161C);

        return Column::make(
            Text::make('Deep links')->style(new Style(fontSize: 24, fontWeight: 700, textColor: 0xFF12161C)),
            Text::make('Launched with')->style($label),
            Text::make($this->initial)->style($value),
            Text::make('Received while running')->style($label),
            Text::make($this->received === [] ? 'nothing yet' : implode("\n", $this->received))->style($value),
            Text::make('Screen reader: '.$this->reader)->style($label),
            Button::make('Announce to the screen reader')->onPress(fn () => Accessibility::announce('Links example is listening')),
        )->style(new Style(flexGrow: 1, padding: 28, gap: 10, backgroundColor: 0xFFF7F8FA));
    }
}

App::run(new Links());
