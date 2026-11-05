<?php

declare(strict_types=1);

use Pam\Desktop\App;
use Pam\Desktop\Attributes\State;
use Pam\Desktop\Component;
use Pam\Desktop\ModalAnimationType;
use Pam\Desktop\ModalPresentation;
use Pam\Desktop\PropKey;
use Pam\Desktop\Style;
use Pam\Desktop\UI\ActivityIndicator;
use Pam\Desktop\UI\BottomSheet;
use Pam\Desktop\UI\Button;
use Pam\Desktop\UI\Column;
use Pam\Desktop\UI\Input;
use Pam\Desktop\UI\Modal;
use Pam\Desktop\UI\Row;
use Pam\Desktop\UI\Text;
use Pam\Desktop\UI\Toggle;

require __DIR__.'/../../vendor/autoload.php';

/**
 * Host-drawn and native controls: switch, activity indicator, masked text
 * fields, a dialog and a bottom sheet.
 */
final class Controls extends Component
{
    #[State]
    public bool $notifications = true;

    #[State]
    public bool $syncing = false;

    #[State]
    public string $phone = '';

    #[State]
    public string $amount = '';

    #[State]
    public bool $dialog = false;

    #[State]
    public bool $sheet = false;

    public function render(): Column
    {
        $label = new Style(fontSize: 13, textColor: 0xFF6B7280);
        $title = new Style(fontSize: 16, fontWeight: 600, textColor: 0xFF111827);
        $card = new Style(padding: 16, gap: 12, borderRadius: 14, backgroundColor: 0xFFFFFFFF, borderWidth: 1, borderColor: 0xFFE5E7EB);
        $field = new Style(height: 44, paddingHorizontal: 12, borderRadius: 10, borderWidth: 1, borderColor: 0xFFD1D5DB, backgroundColor: 0xFFFFFFFF, fontSize: 15, textColor: 0xFF111827);

        return Column::make(
            Text::make('Controls')->style(new Style(fontSize: 26, fontWeight: 700, textColor: 0xFF111827)),

            Column::make(
                Row::make(
                    Column::make(Text::make('Notifications')->style($title), Text::make($this->notifications ? 'On' : 'Off')->style($label))->style(new Style(flexGrow: 1)),
                    Toggle::make($this->notifications)->onToggle(function (string $value): void {
                        $this->notifications = $value === '1';
                    }),
                )->style(new Style(alignItems: \Pam\Desktop\Align::Center)),
                Row::make(
                    Column::make(Text::make('Background sync')->style($title), Text::make($this->syncing ? 'Syncing…' : 'Idle')->style($label))->style(new Style(flexGrow: 1)),
                    ActivityIndicator::make($this->syncing),
                    Toggle::make($this->syncing)->onToggle(function (string $value): void {
                        $this->syncing = $value === '1';
                    }),
                )->style(new Style(alignItems: \Pam\Desktop\Align::Center, gap: 12)),
            )->style($card),

            Column::make(
                Text::make('Phone')->style($label),
                Input::make($this->phone)
                    ->placeholder('(11) 98765-4321')
                    ->property(PropKey::InputFormat, 2)
                    ->property(PropKey::InputFormatPattern, '(##) #####-####')
                    ->onChange(function (string $value): void {
                        $this->phone = $value;
                    })
                    ->style($field),
                Text::make('Amount')->style($label),
                Input::make($this->amount)
                    ->placeholder('R$ 0,00')
                    ->property(PropKey::InputFormat, 3)
                    ->property(PropKey::InputFormatPrefix, 'R$ ')
                    ->property(PropKey::InputFormatLocale, 'pt-BR')
                    ->onChange(function (string $value): void {
                        $this->amount = $value;
                    })
                    ->style($field),
                Text::make("PHP sees: {$this->phone} · {$this->amount}")->style($label),
            )->style($card),

            Row::make(
                Button::make('Open dialog')->onPress(function (): void {
                    $this->dialog = true;
                })->style(new Style(flexGrow: 1)),
                Button::make('Open sheet')->onPress(function (): void {
                    $this->sheet = true;
                })->style(new Style(flexGrow: 1)),
            )->style(new Style(gap: 12)),

            Modal::make(
                Column::make(
                    Text::make('Delete this project?')->style($title),
                    Text::make('This cannot be undone. Press Escape or click outside to keep it.')->style($label),
                    Row::make(
                        Button::make('Keep')->onPress(function (): void {
                            $this->dialog = false;
                        })->style(new Style(flexGrow: 1)),
                        Button::make('Delete')->onPress(function (): void {
                            $this->dialog = false;
                        })->style(new Style(flexGrow: 1, backgroundColor: 0xFFDC2626, textColor: 0xFFFFFFFF, borderRadius: 8)),
                    )->style(new Style(gap: 12)),
                )->style(new Style(width: 340, padding: 20, gap: 12, borderRadius: 16, backgroundColor: 0xFFFFFFFF)),
                $this->dialog,
                ModalPresentation::Dialog,
            )->animationType(ModalAnimationType::Fade)->onRequestClose(function (): void {
                $this->dialog = false;
            }),

            BottomSheet::make(
                Column::make(
                    Text::make('Share with')->style($title),
                    Text::make('A sheet anchored to the bottom of the window.')->style($label),
                    Button::make('Done')->onPress(function (): void {
                        $this->sheet = false;
                    }),
                )->style(new Style(padding: 24, paddingTop: 32, gap: 12, backgroundColor: 0xFFFFFFFF)),
                [0.45],
                0,
                $this->sheet,
            )->onDismiss(function (): void {
                $this->sheet = false;
            }),
        )->style(new Style(padding: 24, gap: 14, flexGrow: 1, backgroundColor: 0xFFF3F4F6));
    }
}

App::run(new Controls());
