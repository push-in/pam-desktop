<?php

declare(strict_types=1);

use Pam\Desktop\App;
use Pam\Desktop\Appearance;
use Pam\Desktop\AppearanceMode;
use Pam\Desktop\Attributes\State;
use Pam\Desktop\Component;
use Pam\Desktop\Platform;
use Pam\Desktop\Storage\Storage;
use Pam\Desktop\Style;
use Pam\Desktop\System\Alert;
use Pam\Desktop\System\AlertButton;
use Pam\Desktop\System\Clipboard;
use Pam\Desktop\System\DeviceInfo;
use Pam\Desktop\System\Timers;
use Pam\Desktop\UI\Button;
use Pam\Desktop\UI\Column;
use Pam\Desktop\UI\Row;
use Pam\Desktop\UI\Text;

require __DIR__.'/../../vendor/autoload.php';

/**
 * Exercises the host modules: timers, durable storage, native dialogs, the
 * clipboard, device information and the appearance preference.
 */
final class Modules extends Component
{
    #[State]
    public int $seconds = 0;

    #[State]
    public int $launches = 0;

    #[State]
    public string $answer = 'No answer yet';

    #[State]
    public string $clipboard = '';

    #[State]
    public string $device = 'Loading device…';

    #[State]
    public bool $dark = false;

    public function mount(): void
    {
        $this->dark = Appearance::isDark();
        Timers::every(1000, function (): void {
            $this->seconds++;
        });
        Storage::get('launches', function (?string $value): void {
            $this->launches = (int) $value + 1;
            Storage::set('launches', (string) $this->launches);
        });
        DeviceInfo::get(function (DeviceInfo $info): void {
            $this->device = sprintf('%s · %s · %s', Platform::os(), $info->locale, $info->timeZone);
        });
    }

    public function render(): Column
    {
        $label = new Style(fontSize: 13, textColor: $this->dark ? 0xFF9AA4B2 : 0xFF5B6472);
        $value = new Style(fontSize: 18, fontWeight: 600, textColor: $this->dark ? 0xFFF4F6F8 : 0xFF12161C);

        return Column::make(
            Text::make('Host modules')->style(new Style(fontSize: 26, fontWeight: 700, textColor: $this->dark ? 0xFFFFFFFF : 0xFF12161C)),
            Text::make($this->device)->style($label),
            Row::make(
                Column::make(Text::make('Timer')->style($label), Text::make("{$this->seconds} s")->style($value))->style(new Style(flexGrow: 1)),
                Column::make(Text::make('Launches')->style($label), Text::make((string) $this->launches)->style($value))->style(new Style(flexGrow: 1)),
            ),
            Text::make('Dialog')->style($label),
            Text::make($this->answer)->style($value),
            Text::make('Clipboard')->style($label),
            Text::make($this->clipboard === '' ? '—' : $this->clipboard)->style($value),
            Row::make(
                Button::make('Ask')->onPress(function (): void {
                    Alert::choose(
                        'Save changes?',
                        'Your edits will be lost if you do not save them.',
                        [AlertButton::cancel('Cancel'), AlertButton::destructive('Discard'), AlertButton::make('Save')],
                        function (int $index): void {
                            $this->answer = ['Cancelled', 'Discarded', 'Saved'][$index] ?? 'Dismissed';
                        },
                    );
                })->style(new Style(flexGrow: 1)),
                Button::make('Copy time')->onPress(function (): void {
                    Clipboard::setText("Timer at {$this->seconds} s", function (): void {
                        Clipboard::getText(function (?string $text): void {
                            $this->clipboard = (string) $text;
                        });
                    });
                })->style(new Style(flexGrow: 1)),
                Button::make($this->dark ? 'Light' : 'Dark')->onPress(function (): void {
                    $this->dark = !$this->dark;
                    Appearance::set($this->dark ? AppearanceMode::Dark : AppearanceMode::Light);
                })->style(new Style(flexGrow: 1)),
            )->style(new Style(gap: 12)),
        )->style(new Style(padding: 24, gap: 10, flexGrow: 1, backgroundColor: $this->dark ? 0xFF14181F : 0xFFF6F7F9));
    }
}

App::run(new Modules());
