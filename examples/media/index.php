<?php

declare(strict_types=1);

use Pam\Desktop\App;
use Pam\Desktop\Attributes\State;
use Pam\Desktop\Component;
use Pam\Desktop\ImageFit;
use Pam\Desktop\MediaReadyEvent;
use Pam\Desktop\Style;
use Pam\Desktop\UI\Button;
use Pam\Desktop\UI\Column;
use Pam\Desktop\UI\MediaPlayer;
use Pam\Desktop\UI\Row;
use Pam\Desktop\UI\Text;

require __DIR__.'/../../vendor/autoload.php';

/**
 * Exercises the media player: a bundled clip with a poster, play and pause
 * from state, seeking, mute, loop and the ready, progress and end events.
 */
final class Media extends Component
{
    #[State]
    public bool $playing = false;

    #[State]
    public bool $muted = true;

    #[State]
    public bool $loop = false;

    #[State]
    public float $seek = 0.0;

    #[State]
    public string $status = 'Not loaded';

    #[State]
    public string $clock = '0.0 / 0.0 s';

    public function render(): Column
    {
        $label = new Style(fontSize: 13, textColor: 0xFF5B6472);

        return Column::make(
            Text::make('MediaPlayer')->style(new Style(fontSize: 24, fontWeight: 700, textColor: 0xFF12161C)),
            MediaPlayer::make('asset://assets/clip.mp4')
                ->thumbnail('asset://assets/poster.jpg')
                ->autoPlay($this->playing)
                ->muted($this->muted)
                ->loop($this->loop)
                ->currentTime($this->seek)
                ->fit(ImageFit::Contain)
                ->nativeRef('clip')
                ->onReady(function (MediaReadyEvent $event): void {
                    $this->status = sprintf('Ready · %d×%d · %.1f s', $event->naturalWidth, $event->naturalHeight, $event->duration);
                })
                ->onProgress(function (float $currentTime, float $duration): void {
                    $this->clock = sprintf('%.1f / %.1f s', $currentTime, $duration);
                })
                ->onEnd(function (): void {
                    $this->playing = false;
                    $this->status = 'Ended';
                })
                ->onError(function (string $message): void {
                    $this->status = 'Error: '.$message;
                })
                ->style(new Style(height: 380, borderRadius: 18, backgroundColor: 0xFF12161C)),
            // Bound to the player by the host: no PHP runs while it plays.
            Row::make(
                Text::make('0:00')->mediaClock('clip', \Pam\Desktop\MediaClockFormat::ElapsedAndDuration)->style(new Style(fontSize: 13, fontWeight: 600, textColor: 0xFF12161C, width: 96)),
                Column::make(
                    Column::make()->mediaProgress('clip')->style(new Style(height: 6, borderRadius: 3, backgroundColor: 0xFF2F6BFF)),
                )->style(new Style(flexGrow: 1, height: 6, borderRadius: 3, backgroundColor: 0xFFE4E7EC, overflow: \Pam\Desktop\Overflow::Hidden)),
            )->style(new Style(gap: 12, alignItems: \Pam\Desktop\Align::Center)),
            Row::make(
                Text::make($this->status)->style(new Style(fontSize: 13, textColor: 0xFF5B6472, flexGrow: 1)),
                Text::make($this->clock)->style($label),
            ),
            Row::make(
                Button::make($this->playing ? 'Pause' : 'Play')->onPress(function (): void { $this->playing = !$this->playing; }),
                Button::make('Restart')->onPress(function (): void { $this->seek = $this->seek === 0.0 ? 0.001 : 0.0; }),
                Button::make('Jump to 5 s')->onPress(function (): void { $this->seek = $this->seek === 5.0 ? 5.001 : 5.0; }),
                Button::make($this->muted ? 'Unmute' : 'Mute')->onPress(function (): void { $this->muted = !$this->muted; }),
                Button::make($this->loop ? 'Loop: on' : 'Loop: off')->onPress(function (): void { $this->loop = !$this->loop; }),
            )->style(new Style(gap: 8)),
        )->style(new Style(flexGrow: 1, padding: 28, gap: 14, backgroundColor: 0xFFF7F8FA));
    }
}

App::run(new Media());
