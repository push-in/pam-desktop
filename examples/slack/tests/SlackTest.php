<?php

declare(strict_types=1);

use App\Screens\Client\ClientScreen;
use App\Services\Workspace;
use Pam\Desktop\App;

use function Pam\Desktop\Testing\mount;

require dirname(__DIR__).'/vendor/autoload.php';

App::components(dirname(__DIR__).'/src', dirname(__DIR__).'/.pam-desktop/components');

$check = static function (bool $condition, string $message): void {
    if (!$condition) {
        fwrite(STDERR, "FAIL {$message}\n");
        exit(1);
    }
};

// The timeline is newest first (an inverted list rests on row 0) and ends
// with the conversation intro, shown at the top.
$workspace = new Workspace();
$timeline = $workspace->timeline('design');
$check($timeline[0]->message?->authorId === 'u-carla', 'the newest message is the first row');
$check(end($timeline)->isIntro(), 'the intro is the last row, at the top');
$check(count(array_filter($timeline, static fn ($entry): bool => $entry->isDay())) === 2, 'one divider per day');

// A message that mentions the reader is highlighted apart from other mentions.
$styles = array_map(static fn ($run): string => $run->style, $workspace->timeline('dm-ana')[0]->runs);
$check(in_array('mention-me', $styles, true) && in_array('link', $styles, true), 'mentions of the reader and links are styled');

// Reactions toggle for the reader; posting adds a message at the bottom.
$mine = static fn (): bool => in_array(Workspace::ME, $workspace->message('design', 's2')?->reactions['👀'] ?? [], true);
$workspace->toggleReaction('design', 's2', '👀');
$check($mine(), 'a reaction adds the reader');
$workspace->toggleReaction('design', 's2', '👀');
$check(!$mine(), 'the same reaction again removes the reader');
$before = count($workspace->messages('random'));
$workspace->post('random', Workspace::ME, 'Count me in!');
$check(count($workspace->messages('random')) === $before + 1, 'a posted message is appended');

// A thread lists its parent, then the replies oldest first.
$thread = $workspace->thread('design', 's4');
$check(count($thread) === 5 && $thread[0]->message?->id === 's4', 'the thread opens with its parent and four replies');

// Search finds channels, people and messages, with the matches marked.
$hits = $workspace->search('ana');
$check(in_array('Ana Ribeiro', array_map(static fn ($hit): string => $hit->label, $hits), true), 'people are found by name');
$onboarding = $workspace->search('onboarding');
$check($onboarding !== [] && in_array('match', array_map(static fn ($run): string => $run->style, $onboarding[0]->runs), true), 'message matches are marked');
$check($workspace->search('a') === [], 'one character searches nothing');

// The client screen, mounted without a host: open a channel, a thread, send.
$client = mount(ClientScreen::class);
$check(str_contains($client->text(), 'Acme Inc') && str_contains($client->text(), 'First pass of the welcome flow'), 'the client opens on #design');
$client->instance->open('engineering');
$client->render();
$check(str_contains($client->text(), 'Release 2.4.0 is tagged'), 'opening a channel shows its messages');
$client->instance->openThread('s14');
$client->render();
$check(str_contains($client->text(), 'Thread'), 'a thread opens beside the channel');
$client->instance->send('Shipping it after lunch');
$client->render();
$check(str_contains($client->text(), 'Shipping it after lunch'), 'a sent message shows in the channel');
$client->unmount();

echo "Slack example tests passed.\n";
