<?php

declare(strict_types=1);

namespace App\Screens\Home;

use App\Lib\Plural;
use App\Services\Greeting;
use Pam\Desktop\Attributes\State;
use Pam\Desktop\Component;

final class HomeScreen extends Component
{
    public string $title = '@NAME@';

    public string $greeting = '';

    #[State]
    public int $count = 0;

    public function mount(): void
    {
        $this->greeting = (new Greeting())->forHour((int) date('G'));
    }

    public function increment(): void
    {
        $this->count++;
    }

    public function clicks(): string
    {
        return Plural::of($this->count, 'click', 'clicks');
    }
}
?>

<template>
    <Column class="home">
        <Column class="content">
            <Text class="eyebrow">{{ $greeting }}</Text>
            <Text class="title">{{ $title }}</Text>
            <Text class="lead">Your native desktop application starts here. Save any file and the window updates in place, keeping its state.</Text>
            <Row class="actions">
                <Button label="Try an interaction" on:press="increment" class="primary" testId="home-increment" />
                <Text class="counter" testId="home-count">{{ clicks() }}</Text>
            </Row>
            <Column class="steps">
                <Text class="steps-title">Make it yours</Text>
                <Row class="step"><Text class="step-path">src/Screens</Text><Text class="step-text">one folder per screen: template, logic and styles together</Text></Row>
                <Row class="step"><Text class="step-path">src/Navigation</Text><Text class="step-text">routes and the screen each one opens</Text></Row>
                <Row class="step"><Text class="step-path">src/Services</Text><Text class="step-text">application logic the screens call</Text></Row>
                <Row class="step"><Text class="step-path">src/Lib</Text><Text class="step-text">small helpers shared by everything</Text></Row>
            </Column>
        </Column>
    </Column>
</template>

<style scoped>
    .home { flex: 1; justify-content: center; align-items: center; padding: 40px; background-color: var(--background); }
    .content { width: 640px; max-width: 100%; gap: 14px; }
    .eyebrow { color: var(--accent); font-size: 12px; font-weight: 700; letter-spacing: 1px; }
    .title { color: var(--text); font-size: 38px; font-weight: 700; line-height: 46px; }
    .lead { color: var(--muted); font-size: 16px; line-height: 25px; }
    .actions { gap: 18px; align-items: center; margin-top: 10px; }
    .primary { background-color: var(--accent); color: #FFFFFF; border-radius: 8px; height: 40px; padding-left: 18px; padding-right: 18px; font-size: 14px; font-weight: 600; }
    .primary:hover { background-color: var(--accent-strong); }
    .counter { color: var(--muted); font-size: 14px; }
    .steps { margin-top: 28px; padding: 20px; gap: 10px; border-radius: 10px; border-width: 1px; border-color: var(--border); background-color: var(--surface); }
    .steps-title { color: var(--text); font-size: 14px; font-weight: 700; }
    .step { gap: 12px; align-items: center; }
    .step-path { width: 130px; color: var(--accent); font-family: monospace; font-size: 13px; }
    .step-text { flex: 1; color: var(--muted); font-size: 13px; line-height: 19px; }
</style>
