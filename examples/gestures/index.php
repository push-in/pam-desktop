<?php

declare(strict_types=1);

use Pam\Desktop\Align;
use Pam\Desktop\App;
use Pam\Desktop\Attributes\State;
use Pam\Desktop\Component;
use Pam\Desktop\GestureEvent;
use Pam\Desktop\GestureType;
use Pam\Desktop\ImageFit;
use Pam\Desktop\Justify;
use Pam\Desktop\Overflow;
use Pam\Desktop\Style;
use Pam\Desktop\UI\Column;
use Pam\Desktop\UI\GestureDetector;
use Pam\Desktop\UI\Image;
use Pam\Desktop\UI\Row;
use Pam\Desktop\UI\Swipeable;
use Pam\Desktop\UI\Text;

require __DIR__.'/../../vendor/autoload.php';

/**
 * Exercises gestures: rows that slide open to reveal actions (a drag with
 * snap points), a photo that pans and zooms natively (drag, Control + wheel
 * or a trackpad pinch), a long press and a swipe.
 */
final class Gestures extends Component
{
    /** @var list<string> */
    #[State]
    public array $messages = ['Invoice 204 is ready', 'Lunch on Friday?', 'Build 2.0.0 passed'];

    #[State]
    public string $status = 'Drag a row to the left, drag or zoom the photo';

    private function row(int $index, string $message): Swipeable
    {
        $action = static fn (string $label, int $color): Column => Column::make(
            Text::make($label)->style(new Style(fontSize: 13, fontWeight: 600, textColor: 0xFFFFFFFF)),
        )->style(new Style(width: 80, height: 56, alignItems: Align::Center, justifyContent: Justify::Center, backgroundColor: $color));

        return Swipeable::make(
            Row::make(
                Text::make($message)->style(new Style(fontSize: 15, textColor: 0xFF12161C)),
            )->style(new Style(height: 56, paddingLeft: 16, alignItems: Align::Center, backgroundColor: 0xFFFFFFFF)),
        )
            ->rightActions(Row::make($action('Archive', 0xFF2F6BFF), $action('Delete', 0xFFE5484D)), 160)
            ->group('inbox')
            ->onOpen(function (string $side) use ($message): void {
                $this->status = "Opened “{$message}” ({$side})";
            })
            ->onClose(function () use ($message): void {
                $this->status = "Closed “{$message}”";
            });
    }

    public function render(): Column
    {
        $rows = [];
        foreach ($this->messages as $index => $message) {
            $rows[] = $this->row($index, $message)->toElement()->key($message);
        }

        return Column::make(
            Text::make('Gestures')->style(new Style(fontSize: 24, fontWeight: 700, textColor: 0xFF12161C)),
            Row::make(
                Column::make(
                    Column::make(...$rows)->style(new Style(borderRadius: 14, overflow: Overflow::Hidden, gap: 1, backgroundColor: 0xFFE4E7EC)),
                    GestureDetector::make(
                        GestureType::LongPress,
                        Text::make('Hold me for half a second')->style(new Style(fontSize: 14, textColor: 0xFFFFFFFF, padding: 14)),
                    )
                        ->onBegin(function (GestureEvent $event): void {
                            $this->status = 'Long press began';
                        })
                        ->onEnd(function (GestureEvent $event): void {
                            $this->status = 'Long press ended';
                        })
                        ->style(new Style(borderRadius: 12, backgroundColor: 0xFF12161C, alignItems: Align::Center)),
                    GestureDetector::make(
                        GestureType::Swipe,
                        Text::make('Swipe me in any direction')->style(new Style(fontSize: 14, textColor: 0xFFFFFFFF, padding: 14)),
                    )
                        ->onEnd(function (GestureEvent $event): void {
                            $this->status = sprintf('Swiped %d, %d', $event->translationX, $event->translationY);
                        })
                        ->style(new Style(borderRadius: 12, backgroundColor: 0xFF12B76A, alignItems: Align::Center)),
                )->style(new Style(width: 360, gap: 12)),
                GestureDetector::make(
                    GestureType::Pan,
                    GestureDetector::make(
                        GestureType::Pinch,
                        Image::make('asset://photo.jpg')->fit(ImageFit::Cover)->style(new Style(width: 400, height: 300)),
                    )->nativeTransform(maximumScale: 5, focalZoom: true, bounds: true),
                )
                    ->nativeTransform(maximumScale: 5, bounds: true)
                    ->onEnd(function (GestureEvent $event): void {
                        $this->status = sprintf('Photo at scale %.2f', $event->nativeScale ?? 1.0);
                    })
                    ->cursor('grab')
                    ->style(new Style(width: 400, height: 300, borderRadius: 16, overflow: Overflow::Hidden, backgroundColor: 0xFF12161C)),
            )->style(new Style(gap: 24)),
            Text::make($this->status)->style(new Style(fontSize: 13, textColor: 0xFF5B6472)),
        )->style(new Style(flexGrow: 1, padding: 28, gap: 16, backgroundColor: 0xFFF7F8FA));
    }
}

App::run(new Gestures());
