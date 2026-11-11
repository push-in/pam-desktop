<?php

declare(strict_types=1);

namespace App\Components;

use Pam\Desktop\Attributes\State;
use Pam\Desktop\Component;

final class Composer extends Component
{
    /** @var list<string|null> null draws a divider */
    public array $formatting = ['bold', 'italic', 'strikethrough', null, 'link', null, 'list-ordered', 'list', null, 'code', 'square-code'];

    /** @var list<string|null> */
    public array $tools = ['type', 'smile', 'at-sign', null, 'video', 'mic', null, 'square-slash'];

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
            <View p-for="($icon, $i) in $formatting" p-key="$i" :class="$icon === null ? 'divider' : 'button'">
                <Icon p-if="$icon !== null" :name="$icon" font="Lucide" size="16" class="format" />
            </View>
        </Row>
        <Input testId="composer" class="field" multiline="true" submitBehavior="submit" model="draft" sync="immediate"
               :placeholder="$placeholder" placeholderColor="#868686" @submit="send" />
        <Row class="actions">
            <Column class="attach"><Icon name="plus" font="Lucide" size="16" class="attach-icon" /></Column>
            <View p-for="($icon, $i) in $tools" p-key="$i" :class="$icon === null ? 'divider' : 'button'">
                <Icon p-if="$icon !== null" :name="$icon" font="Lucide" size="16" class="tool" />
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
</style>
