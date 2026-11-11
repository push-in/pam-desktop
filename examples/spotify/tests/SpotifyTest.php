<?php

declare(strict_types=1);

use App\Screens\Player\PlayerScreen;
use App\Services\Library;
use App\Services\Player;
use Pam\Desktop\App;
use Pam\DesktopUi\DesktopUI;

use function Pam\Desktop\Testing\mount;

require dirname(__DIR__).'/vendor/autoload.php';

DesktopUI::boot(dirname(__DIR__).'/.pam-desktop/desktop-ui');
App::components(dirname(__DIR__).'/src', dirname(__DIR__).'/.pam-desktop/components');

$check = static function (bool $condition, string $message): void {
    if (!$condition) {
        fwrite(STDERR, "FAIL {$message}\n");
        exit(1);
    }
};

// The player reads its position from a clock, so the test drives time.
$library = new Library();
$now = 1000.0;
$player = new Player($library, static function () use (&$now): float { return $now; });
$check(!$player->playing() && $player->track()?->id === 't9' && $player->position() === 74.0, 'the app opens paused inside Discover Weekly');

$player->play($library->playlist('release'), 't19');
$check($player->playing() && $player->track()?->title === 'Sugar Static', 'playing a track of a playlist starts it');
$now += 30;
$check($player->position() === 30.0, 'the position follows the clock while playing');
$player->toggle();
$now += 100;
$check($player->position() === 30.0, 'a paused track keeps its position');
$player->toggle();
$player->seek(181.0);
$now += 5;
$check($player->advance() && $player->track()?->id === 't22' && $player->position() === 0.0, 'a finished track moves on to the next');

$player->previous();
$check($player->track()?->id === 't19', 'previous in the first seconds goes back a track');
$now += 10;
$player->previous();
$check($player->track()?->id === 't19' && $player->position() === 0.0, 'previous later restarts the track');

$player->cycleRepeat();
$player->cycleRepeat();
$player->next();
$check($player->repeat === 'one' && $player->track()?->id === 't19', 'repeat one replays the same track');

$liked = $player->isLiked('t19');
$player->toggleLike('t19');
$check($player->isLiked('t19') !== $liked, 'a like toggles');
$player->setVolume(0.0);
$check($player->volumeIcon() === 'volume-x' && $player->muted, 'zero volume mutes');

$found = $library->search('vera');
$check($found['playlists'][0]->name === 'Vera Nox' && count($found['tracks']) === 3, 'search finds the artist and their songs');

// The screen, mounted without a host.
$screen = mount(PlayerScreen::class);
$check(str_contains($screen->text(), 'Made For David') && str_contains($screen->text(), 'Your Library'), 'the app opens on Home');
$screen->instance->open('discover');
$screen->render();
$check(str_contains($screen->text(), 'Discover Weekly') && str_contains($screen->text(), '10 songs, about 37 min'), 'a playlist opens with its summary');
$screen->instance->play('discover', 't4');
$screen->render();
$check($screen->instance->player->track()?->title === 'Glass Houses', 'a track plays from the playlist');
$screen->instance->query = 'lofi';
$screen->instance->search();
$screen->render();
$check(str_contains($screen->text(), 'lofi beats'), 'search shows the matching playlist');
$screen->unmount();

echo "Spotify example tests passed.\n";
