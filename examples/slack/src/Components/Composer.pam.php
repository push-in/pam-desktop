<?php

declare(strict_types=1);

namespace App\Components;

use Pam\Desktop\Attributes\State;
use Pam\Desktop\Component;

final class Composer extends Component
{
    /** @var list<array{0: string, 1: string, 2: ?string}|null> icon, label, shortcut; null draws a divider */
    public array $formatting = [
        ['bold', 'Bold', 'Ctrl+B'], ['italic', 'Italic', 'Ctrl+I'], ['strikethrough', 'Strikethrough', 'Ctrl+Shift+X'], null,
        ['link', 'Link', 'Ctrl+Shift+U'], null,
        ['list-ordered', 'Ordered list', 'Ctrl+Shift+7'], ['list', 'Bulleted list', 'Ctrl+Shift+8'], null,
        ['code', 'Code', 'Ctrl+Shift+C'], ['square-code', 'Code block', 'Ctrl+Alt+Shift+C'],
    ];

    /** @var list<array{0: string, 1: string, 2: ?string}|null> */
    public array $tools = [
        ['type', 'Hide formatting', null], ['smile', 'Emoji', null], ['at-sign', 'Mention someone', null], null,
        ['video', 'Record video clip', null], ['mic', 'Record audio clip', null], null,
        ['square-slash', 'Run shortcut', null],
    ];

    #[State(hotReload: false)]
    public string $draft = '';

    public function __construct(public string $placeholder)
    {
    }

    public function send(): void
    {
        $text = trim($this->draft);
        if ($text === '') {
            return;
        }
        $this->emit('send', $text);
        $this->draft = '';
    }

    public function ready(): bool
    {
        return trim($this->draft) !== '';
    }
}
?>

<template>
    <Column class="composer">
        <Row class="toolbar">
            <View p-for="($item, $i) in $formatting" p-key="$i" :class="$item === null ? 'divider' : 'slot'">
                <Tooltip p-if="$item !== null" :title="$item[1]" :shortcut="$item[2]" placement="top">
                    <View class="button"><Icon :name="$item[0]" font="Lucide" size="16" class="format" /></View>
                </Tooltip>
            </View>
        </Row>
        <Input testId="composer" class="field" multiline="true" submitBehavior="submit" model="draft" sync="immediate"
               :placeholder="$placeholder" placeholderColor="#868686" @submit="send" />
        <Row class="actions">
            <Column class="attach"><Icon name="plus" font="Lucide" size="16" class="attach-icon" /></Column>
            <View p-for="($item, $i) in $tools" p-key="$i" :class="$item === null ? 'divider' : 'slot'">
                <Tooltip p-if="$item !== null" :title="$item[1]" :shortcut="$item[2]" placement="top">
                    <View class="button"><Icon :name="$item[0]" font="Lucide" size="16" class="tool" /></View>
                </Tooltip>
            </View>
            <View class="spacer" />
            <Row :class="ready() ? 'send ready' : 'send'">
                <Pressable class="send-button" cursor="pointer" @press="send">
                    <Icon name="send-horizontal" font="Lucide" size="16" :class="ready() ? 'send-icon ready-icon' : 'send-icon'" />
                </Pressable>
                <View class="send-split" />
                <Column class="send-more">
                    <Icon name="chevron-down" font="Lucide" size="14" :class="ready() ? 'send-icon ready-icon' : 'send-icon'" />
                </Column>
            </Row>
        </Row>
    </Column>
</template>

<style scoped>
    .composer { margin: 0 20px 20px 20px; border-radius: 8px; border-width: 1px; border-color: #1D1C1D4D; }
    .composer:focus-within { border-color: #1D1C1D80; }
    .toolbar { height: 36px; padding: 0 6px; gap: 2px; align-items: center; background-color: #F8F8F8;
               border-top-left-radius: 8px; border-top-right-radius: 8px; }
    .button { width: 28px; height: 28px; border-radius: 4px; align-items: center; justify-content: center; }
    .button:hover { background-color: #1D1C1D0F; }
    .format { color: #1D1C1D80; }
    .divider { width: 1px; height: 20px; margin: 0 4px; background-color: var(--faint); }
    .field { min-height: 22px; max-height: 240px; padding: 8px 12px; border-width: 0; font-size: 15px; line-height: 22px;
             background-color: #00000000; }
    .actions { height: 40px; padding: 0 6px; gap: 2px; align-items: center; }
    .attach { width: 28px; height: 28px; border-radius: 14px; margin-right: 4px; background-color: #1D1C1D0F;
              align-items: center; justify-content: center; }
    .attach-icon { color: #1D1C1DB3; }
    .tool { color: #1D1C1DB3; }
    .spacer { flex: 1; }
    .send { height: 28px; align-items: center; border-radius: 4px; }
    .ready { background-color: var(--send); }
    .send-button { width: 32px; height: 28px; align-items: center; justify-content: center; }
    .send-split { width: 1px; height: 20px; background-color: #FFFFFF40; }
    .send-more { width: 22px; height: 28px; align-items: center; justify-content: center; }
    .send-icon { color: #1D1C1D59; }
    .ready-icon { color: #FFFFFF; }
    .slot { flex-direction: row; }
</style>
