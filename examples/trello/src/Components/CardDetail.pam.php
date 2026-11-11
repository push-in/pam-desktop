<?php

declare(strict_types=1);

namespace App\Components;

use App\Domain\Card;
use App\Services\Board;
use Pam\Desktop\Attributes\State;
use Pam\Desktop\Component;

final class CardDetail extends Component
{
    /** @var list<array{icon: string, label: string}> */
    public array $actions = [
        ['icon' => 'user', 'label' => 'Members'], ['icon' => 'tag', 'label' => 'Labels'],
        ['icon' => 'square-check-big', 'label' => 'Checklist'], ['icon' => 'clock-3', 'label' => 'Dates'],
        ['icon' => 'paperclip', 'label' => 'Attachment'], ['icon' => 'panel-top', 'label' => 'Cover'],
        ['icon' => 'arrow-right', 'label' => 'Move'], ['icon' => 'copy', 'label' => 'Copy'],
        ['icon' => 'archive', 'label' => 'Archive'],
    ];

    #[State]
    public int $revision = 0;

    public function __construct(public Board $board, public string $cardId)
    {
    }

    public function card(): Card
    {
        return $this->board->card($this->cardId);
    }

    public function listTitle(): string
    {
        return $this->board->listOf($this->cardId)?->title ?? '';
    }

    public function close(): void
    {
        $this->emit('close');
    }

    public function complete(): void
    {
        $this->board->toggleComplete($this->cardId);
        $this->revision++;
        $this->emit('changed');
    }

    public function watch(): void
    {
        $this->board->toggleWatch($this->cardId);
        $this->revision++;
        $this->emit('changed');
    }

    /** The checklist as a share, for its progress bar. */
    public function progress(): float
    {
        $checklist = $this->card()->checklist;

        return $checklist === null || $checklist[1] === 0 ? 0.0 : $checklist[0] / $checklist[1];
    }

    public function percent(): string
    {
        return (int) round($this->progress() * 100).'%';
    }

    public function dueText(): string
    {
        return $this->card()->due?->format('M j, g:i A') ?? '';
    }
}
?>

<template>
    <Modal visible="true" presentation="dialog" backdropColor="#7A091E42" on:requestClose="close">
        <Column class="dialog">
            <Image p-if="card()->cover !== null" :source="card()->cover" class="cover" />
            <Pressable class="close" cursor="pointer" @press="close" accessibilityLabel="Close">
                <Icon name="x" font="Lucide" size="20" class="subtle" />
            </Pressable>
            <ScrollView class="scroll" scrollbarWidth="thin">
                <Row class="body">
                    <Column class="main">
                        <Row class="heading">
                            <Icon name="credit-card" font="Lucide" size="20" class="subtle heading-icon" />
                            <Column class="heading-text">
                                <Text class="title">{{ card()->title }}</Text>
                                <Text class="in-list">in list <Span class="list-name">{{ listTitle() }}</Span></Text>
                            </Column>
                        </Row>

                        <Row class="facts">
                            <Column p-if="card()->memberIds !== []" class="fact">
                                <Text class="fact-label">Members</Text>
                                <Row class="faces">
                                    <Text p-for="$id in card()->memberIds" p-key="$id" class="face"
                                          :backgroundColor="$board->member($id)->color">{{ $board->member($id)->initials() }}</Text>
                                    <View class="face-add"><Icon name="plus" font="Lucide" size="16" class="subtle" /></View>
                                </Row>
                            </Column>
                            <Column p-if="card()->labels !== []" class="fact">
                                <Text class="fact-label">Labels</Text>
                                <Row class="chips">
                                    <Text p-for="$label in card()->labels" p-key="$label->name" class="chip"
                                          :backgroundColor="$label->color->value">{{ $label->name }}</Text>
                                </Row>
                            </Column>
                            <Column class="fact">
                                <Text class="fact-label">Notifications</Text>
                                <Pressable :class="card()->watching ? 'button button-on' : 'button'" cursor="pointer" @press="watch">
                                    <Icon name="eye" font="Lucide" size="16" class="text" />
                                    <Text class="button-text">{{ card()->watching ? 'Watching' : 'Watch' }}</Text>
                                    <Icon p-if="card()->watching" name="check" font="Lucide" size="16" class="text" />
                                </Pressable>
                            </Column>
                            <Column p-if="card()->due !== null" class="fact">
                                <Text class="fact-label">Due date</Text>
                                <Row class="due">
                                    <Pressable class="checkbox" cursor="pointer" @press="complete" accessibilityRole="checkbox">
                                        <Icon :name="card()->complete ? 'square-check' : 'square'" font="Lucide" size="18"
                                              :class="card()->complete ? 'checked' : 'subtle'" />
                                    </Pressable>
                                    <Text class="due-text">{{ dueText() }}</Text>
                                    <Text p-if="card()->complete" class="pill pill-done">Complete</Text>
                                    <Text p-else-if="card()->dueState($board->today()) === 'overdue'" class="pill pill-overdue">Overdue</Text>
                                    <Text p-else-if="card()->dueState($board->today()) === 'soon'" class="pill pill-soon">Due soon</Text>
                                </Row>
                            </Column>
                        </Row>

                        <Row class="section">
                            <Icon name="text" font="Lucide" size="20" class="subtle heading-icon" />
                            <Text class="section-title">Description</Text>
                        </Row>
                        <Text p-if="card()->description !== ''" class="description">{{ card()->description }}</Text>
                        <Text p-else class="placeholder">Add a more detailed description…</Text>

                        <Column p-if="card()->checklist !== null" class="checklist">
                            <Row class="section">
                                <Icon name="square-check-big" font="Lucide" size="20" class="subtle heading-icon" />
                                <Text class="section-title">Checklist</Text>
                            </Row>
                            <Row class="progress">
                                <Text class="percent">{{ percent() }}</Text>
                                <Row class="bar">
                                    <View :flexGrow="progress()" :class="progress() >= 1 ? 'fill fill-done' : 'fill'" />
                                    <View :flexGrow="1 - progress()" />
                                </Row>
                            </Row>
                        </Column>

                        <Row class="section">
                            <Icon name="list" font="Lucide" size="20" class="subtle heading-icon" />
                            <Text class="section-title">Activity</Text>
                        </Row>
                        <Row class="comment">
                            <Text class="face" backgroundColor="#0C66E4">DB</Text>
                            <Text class="comment-box">Write a comment…</Text>
                        </Row>
                    </Column>

                    <Column class="side">
                        <Text class="fact-label">Add to card</Text>
                        <Row p-for="$action in array_slice($actions, 0, 6)" p-key="$action['label']" class="side-button">
                            <Icon :name="$action['icon']" font="Lucide" size="16" class="text" />
                            <Text class="button-text">{{ $action['label'] }}</Text>
                        </Row>
                        <Text class="fact-label side-gap">Actions</Text>
                        <Row p-for="$action in array_slice($actions, 6)" p-key="$action['label']" class="side-button">
                            <Icon :name="$action['icon']" font="Lucide" size="16" class="text" />
                            <Text class="button-text">{{ $action['label'] }}</Text>
                        </Row>
                    </Column>
                </Row>
            </ScrollView>
        </Column>
    </Modal>
</template>

<style scoped>
    .dialog { width: 768px; max-height: 720px; border-radius: 12px; background-color: #F1F2F4; overflow: hidden; }
    .cover { height: 160px; }
    .close { position: absolute; top: 8px; right: 8px; width: 40px; height: 40px; border-radius: 20px;
             background-color: #F1F2F4; align-items: center; justify-content: center; z-index: 2; }
    .close:hover { background-color: #DCDFE4; }
    .scroll { max-height: 560px; }
    .body { gap: 16px; padding: 20px 16px 24px 16px; }
    .main { flex: 1; gap: 8px; }
    .heading { gap: 12px; align-items: flex-start; padding-right: 32px; }
    .heading-icon { margin-top: 4px; }
    .heading-text { flex: 1; gap: 2px; }
    .title { font-size: 20px; font-weight: 600; line-height: 28px; }
    .in-list { font-size: 14px; color: var(--subtle); }
    .list-name { text-decoration-line: underline; }
    .facts { gap: 16px; flex-wrap: wrap; padding: 8px 0 8px 32px; }
    .fact { gap: 6px; }
    .fact-label { font-size: 12px; font-weight: 600; color: var(--subtle); }
    .faces { gap: 4px; }
    .face { width: 32px; height: 32px; border-radius: 16px; font-size: 12px; font-weight: 700; color: #FFFFFF;
            text-align: center; line-height: 32px; }
    .face-add { width: 32px; height: 32px; border-radius: 16px; background-color: var(--neutral); align-items: center; justify-content: center; }
    .chips { gap: 4px; }
    .chip { height: 32px; padding: 0 12px; border-radius: 3px; font-size: 14px; font-weight: 500; line-height: 32px; }
    .button { flex-direction: row; height: 32px; gap: 8px; padding: 0 12px; align-items: center; border-radius: 3px; background-color: var(--neutral); }
    .button:hover { background-color: var(--neutral-hover); }
    .button-on { background-color: var(--neutral-hover); }
    .button-text { font-size: 14px; font-weight: 500; }
    .due { height: 32px; gap: 8px; padding: 0 8px; align-items: center; border-radius: 3px; background-color: var(--neutral); }
    .checkbox { width: 20px; height: 20px; align-items: center; justify-content: center; }
    .checked { color: var(--brand); }
    .due-text { font-size: 14px; }
    .pill { height: 20px; padding: 0 4px; border-radius: 3px; font-size: 12px; line-height: 20px; }
    .pill-done { background-color: var(--success); color: #FFFFFF; }
    .pill-overdue { background-color: var(--danger); color: #FFFFFF; }
    .pill-soon { background-color: var(--warning); }
    .section { gap: 12px; align-items: center; padding-top: 16px; }
    .section-title { font-size: 16px; font-weight: 600; }
    .description { padding-left: 32px; font-size: 14px; line-height: 20px; }
    .placeholder { margin-left: 32px; padding: 8px 12px; min-height: 56px; border-radius: 3px; background-color: var(--neutral);
                   font-size: 14px; color: var(--subtle); }
    .checklist { gap: 8px; }
    .progress { gap: 12px; align-items: center; padding-left: 0; }
    .percent { width: 20px; font-size: 11px; color: var(--subtle); }
    .bar { flex: 1; flex-direction: row; height: 8px; border-radius: 4px; background-color: var(--neutral-hover); overflow: hidden; }
    .fill { border-radius: 4px; background-color: var(--brand); }
    .fill-done { background-color: var(--success); }
    .comment { gap: 12px; align-items: center; }
    .comment-box { flex: 1; height: 36px; padding: 0 12px; border-radius: 8px; background-color: var(--card); font-size: 14px;
                   color: var(--subtle); line-height: 36px; box-shadow: 0 1px 1px #091E4240; }
    .side { width: 168px; gap: 8px; padding-top: 52px; }
    .side-gap { padding-top: 16px; }
    .side-button { height: 32px; gap: 8px; padding: 0 12px; align-items: center; border-radius: 3px; background-color: var(--neutral); }
    .side-button:hover { background-color: var(--neutral-hover); }
    .subtle { color: var(--subtle); }
    .text { color: var(--text); }
</style>
