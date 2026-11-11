<?php

declare(strict_types=1);

namespace App\Services;

use App\Domain\Message;
use DateTimeImmutable;

/** The conversations the workspace opens with, dated relative to today. */
final class Seed
{
    private int $sequence = 0;

    public function into(Workspace $workspace): void
    {
        $this->conversation($workspace, 'design', [
            ['-1 day 16:02', 'u-carla', 'Kicking off the *Q4 onboarding* refresh. Brief and goals are in the canvas, feedback welcome before Thursday :)'],
            ['-1 day 16:05', 'u-ana', 'On it! I will start from the empty states, they are the weakest part today.', ['👀' => ['u-carla', 'u-bruno']]],
            ['-1 day 16:06', 'u-ana', 'Also pulling the latest research notes from https://notion.so/acme/onboarding-research'],
            ['09:12', 'u-ana', 'First pass of the welcome flow is up for review. Three screens, light and dark, all with real copy.', ['🔥' => ['u-me', 'u-carla', 'u-bruno'], '🙌' => ['u-erica']], 4, ['u-carla', 'u-me', 'u-bruno'], '11:40'],
            ['09:14', 'u-ana', '@Bruno Costa can you check if the illustration sizes work with the new `ImageFit::Cover` crops?'],
            ['09:31', 'u-bruno', 'Looks great. The crops hold up down to 900px wide, below that the hero gets tight. I would drop the subtitle on compact windows.', ['👍' => ['u-ana', 'u-me']]],
            ['10:02', 'u-me', 'Love the direction. Shipping this behind the `onboarding_v2` flag first so we can compare activation week over week.', ['✅' => ['u-carla']]],
            ['10:03', 'u-me', 'I will wire the analytics events today.'],
            ['11:47', 'u-carla', 'Reminder: design critique moved to *3 PM* in the Huddle room. Bring the motion studies!', ['📌' => ['u-ana', 'u-me', 'u-erica', 'u-bruno']]],
        ]);
        $this->thread($workspace, 's4', [
            ['09:40', 'u-carla', 'The second screen is so much clearer than before. Can we try the illustration on the left instead?'],
            ['10:05', 'u-me', 'Agree with Carla. Also the CTA could say *Get started* instead of _Continue_.', ['💯' => ['u-ana']]],
            ['10:52', 'u-bruno', 'Motion on the progress dots is lovely. Is it the spring from the design system?'],
            ['11:40', 'u-ana', 'Yes! Same spring as the sheets. Updated both screens with your notes, thanks all 🙏', ['❤️' => ['u-carla', 'u-me']]],
        ]);
        $this->conversation($workspace, 'engineering', [
            ['08:40', 'u-bruno', 'Release *2.4.0* is tagged. Changelog and rollout plan in the thread.', ['🚀' => ['u-me', 'u-diego', 'u-felipe']], 6, ['u-diego', 'u-me'], '09:58'],
            ['09:02', 'u-diego', 'iOS build is green, submitting to TestFlight now.'],
            ['09:20', 'u-felipe', 'Dashboards show p95 latency down 18% since the deploy. Nice work everyone.', ['🎉' => ['u-bruno', 'u-me', 'u-ana']]],
            ['10:15', 'u-bruno', '@David Balbino can you take a look at the flaky `sync_test` before lunch?'],
        ]);
        $this->conversation($workspace, 'general', [
            ['-1 day 09:00', 'u-erica', 'Welcome to the team, @Felipe Rocha! 👋', ['👋' => ['u-me', 'u-ana', 'u-bruno', 'u-carla', 'u-diego']]],
            ['08:30', 'u-carla', 'All-hands tomorrow at 10 AM. Agenda: Q4 goals, the new office and a demo from the design team.'],
        ]);
        $this->conversation($workspace, 'random', [
            ['12:05', 'u-diego', 'Who is up for ramen at 12:30?', ['🍜' => ['u-bruno', 'u-felipe']]],
        ]);
        $this->conversation($workspace, 'dm-ana', [
            ['-1 day 18:20', 'u-me', 'Thanks for the walkthrough today, super clear.'],
            ['-1 day 18:22', 'u-ana', 'Anytime! I will send the Figma link tomorrow morning.'],
            ['09:05', 'u-ana', 'Here it is: https://figma.com/file/acme-onboarding. Let me know what you think about the second screen @David Balbino'],
        ]);
        $this->conversation($workspace, 'dm-bruno', [
            ['10:20', 'u-bruno', 'Pairing on the sync bug after lunch?'],
            ['10:21', 'u-me', 'Sure, 2 PM works.'],
        ]);
    }

    /** @param list<array{0: string, 1: string, 2: string, 3?: array<string, list<string>>}> $rows */
    private function thread(Workspace $workspace, string $parentId, array $rows): void
    {
        foreach ($rows as $row) {
            [$when, $author, $text, $reactions] = $row + [3 => []];
            $workspace->appendReply($parentId, new Message('s'.++$this->sequence, $author, new DateTimeImmutable($when), $text, $reactions));
        }
    }

    /** @param list<array{0: string, 1: string, 2: string, 3?: array<string, list<string>>, 4?: int, 5?: list<string>, 6?: string}> $rows */
    private function conversation(Workspace $workspace, string $id, array $rows): void
    {
        foreach ($rows as $row) {
            [$when, $author, $text, $reactions, $replies, $repliedBy, $lastReply] = $row + [3 => [], 4 => 0, 5 => [], 6 => null];
            $workspace->append($id, new Message(
                's'.++$this->sequence,
                $author,
                new DateTimeImmutable($when),
                $text,
                $reactions,
                $replies,
                $repliedBy,
                $lastReply === null ? null : new DateTimeImmutable($lastReply),
            ));
        }
    }
}
