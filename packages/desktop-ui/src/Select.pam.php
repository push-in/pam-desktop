<?php

declare(strict_types=1);

namespace Pam\DesktopUi;

use Closure;
use InvalidArgumentException;
use Pam\Desktop\Attributes\State;
use Pam\Desktop\Component;
use Pam\Desktop\I18n\FrameworkText;
use Pam\Desktop\KeyEvent;
use Pam\Desktop\System\ViewRef;
use Pam\Desktop\ViewMeasurement;

/** Controlled native combobox; install SelectProvider at the current window/modal root. */
final class Select extends Component
{
    use Concerns\LocalizedText;

    #[State] public bool $open = false;
    #[State] public string $query = '';
    /** @var list<SelectOption> */
    #[State] public array $visibleOptions = [];
    #[State] public int $highlighted = -1;
    #[State] public float $popupX = 0;
    #[State] public float $popupY = 0;
    #[State] public float $popupWidth = 0;
    #[State] public float $popupHeight = 0;
    #[State] public int $scrollRevision = 0;
    #[State] public float $scrollOffset = 0;
    use Concerns\FollowsSelectTheme;
    public readonly string $reference;
    /** Trigger geometry and typography, so products can match their input fields. */
    public readonly AutocompleteInputStyle $fieldStyle;
    private int $generation = 0;

    /** @param list<SelectOption> $options @param Closure(int|string|null): void $changed */
    public function __construct(
        public array $options,
        #[State] public int|string|null $value,
        private readonly Closure $changed,
        #[State] public bool $enabled = true,
        public ?string $placeholder = null,
        public string $host = 'desktop-ui',
        public ?string $label = null,
        SelectTheme|Closure|null $theme = null,
        public bool $showClear = false,
        public bool $filter = false,
        public ?string $filterPlaceholder = null,
        public ?string $emptyMessage = null,
        ?AutocompleteInputStyle $fieldStyle = null,
    ) {
        $this->placeholder ??= FrameworkText::get('select.placeholder');
        $this->label ??= FrameworkText::get('select.label');
        $this->filterPlaceholder ??= FrameworkText::get('select.search');
        $this->emptyMessage ??= FrameworkText::get('select.empty');
        $this->followTheme($theme);
        $this->fieldStyle = $fieldStyle ?? new AutocompleteInputStyle(40, 14, 400, 12, 6);
        $this->options = self::validatedOptions($options);
        $this->visibleOptions = $this->options;
        $this->reference = 'select-'.spl_object_id($this);
        ViewRef::of($host);
    }

    /** @param list<SelectOption> $options */
    public function setOptions(array $options): void
    {
        $this->options = self::validatedOptions($options);
        $this->search($this->query);
    }

    /** @param list<SelectOption> $options @return list<SelectOption> */
    private static function validatedOptions(array $options): array
    {
        if (count($options) > 10_000) throw new InvalidArgumentException('Select accepts at most 10000 local options; paginate larger catalogues.');
        $keys = [];
        foreach ($options as $option) {
            if (!$option instanceof SelectOption) throw new InvalidArgumentException('Select options must be SelectOption instances.');
            if (isset($keys[$option->key])) throw new InvalidArgumentException('Select option values must be unique.');
            $keys[$option->key] = true;
        }
        return array_values($options);
    }

    public function selectedLabel(): string
    {
        foreach ($this->options as $option) if ($option->value === $this->value) return $option->label;
        return $this->placeholder;
    }

    public function toggle(): void
    {
        if ($this->open) { $this->close(); return; }
        if (!$this->enabled || $this->options === []) return;
        $this->search('');
        $generation = ++$this->generation;
        ViewRef::of($this->reference)->measureInWindow(function (?ViewMeasurement $anchor) use ($generation): void {
            if ($generation !== $this->generation || $anchor === null) return;
            ViewRef::of($this->host)->measureInWindow(function (?ViewMeasurement $host) use ($generation, $anchor): void {
                if ($generation !== $this->generation || $host === null || !$this->enabled) return;
                $this->place($anchor, $host);
                $this->highlighted = $this->firstEnabled();
                foreach ($this->options as $index => $option) {
                    if ($option->value === $this->value && !$option->disabled) $this->highlighted = $index;
                }
                $this->open = true;
                ViewRef::of($this->filter ? $this->reference.'-search' : $this->reference)->focus();
                $this->scrollToHighlighted();
            });
        });
    }

    /** Geometry uses the provider's coordinate space, including nested dialogs and scrolling. */
    public function place(ViewMeasurement $anchor, ViewMeasurement $host): void
    {
        $this->popupWidth = min($anchor->width, max(0, $host->width - 8));
        $this->popupX = max(4, min($anchor->windowX - $host->windowX, $host->width - $this->popupWidth - 4));
        $below = $anchor->windowY - $host->windowY + $anchor->height + 4;
        $above = $anchor->windowY - $host->windowY - 4;
        $desired = min(280, count($this->options) * 38 + 8 + ($this->filter ? 44 : 0));
        $useBelow = $host->height - $below - 4 >= $desired || $host->height - $below - 4 >= $above - 4;
        $this->popupHeight = max(0, min($desired, $useBelow ? $host->height - $below - 4 : $above - 4));
        $this->popupY = $useBelow ? $below : $above - $this->popupHeight;
    }

    public function close(): void
    {
        ++$this->generation;
        if (!$this->open) return;
        $this->open = false;
        ViewRef::of($this->reference)->focus();
    }

    public function choose(int|string $value): void
    {
        if (!$this->open || !$this->enabled) return;
        foreach ($this->options as $option) {
            if ($option->value !== $value || $option->disabled) continue;
            $this->close();
            ($this->changed)($value);
            return;
        }
    }

    public function clear(): void
    {
        if (!$this->enabled || !$this->showClear || $this->value === null) return;
        $this->close();
        ($this->changed)(null);
    }

    public function search(string $query): void
    {
        $this->options = self::validatedOptions($this->options);
        $this->query = mb_substr($query, 0, 256);
        $needle = mb_strtolower(trim($this->query));
        $this->visibleOptions = $needle === '' ? $this->options : array_values(array_filter(
            $this->options, static fn (SelectOption $option): bool => str_contains(mb_strtolower($option->label), $needle),
        ));
        $this->highlighted = $this->firstEnabled();
        $this->scrollOffset = 0;
        ++$this->scrollRevision;
    }

    public function key(KeyEvent|string $payload): void
    {
        $event = $payload instanceof KeyEvent ? $payload : KeyEvent::fromPayload($payload);
        if (!$this->enabled || $event->control || $event->meta || $event->alt) return;
        if ($event->key === 'Escape' || $event->key === 'Tab') { $this->close(); return; }
        if (!$this->open) {
            if (in_array($event->key, ['Enter', ' ', 'ArrowDown', 'ArrowUp'], true)) $this->toggle();
            return;
        }
        if ($event->key === 'Enter' || $event->key === ' ') {
            $option = $this->visibleOptions[$this->highlighted] ?? null;
            if ($option !== null) $this->choose($option->value);
            return;
        }
        $count = count($this->visibleOptions);
        $direction = $event->key === 'ArrowUp' || $event->key === 'End' ? -1 : 1;
        if ($event->key === 'Home') $this->highlighted = -1;
        elseif ($event->key === 'End') $this->highlighted = $count;
        elseif (!in_array($event->key, ['ArrowDown', 'ArrowUp'], true)) return;
        for ($step = 0; $step < $count; $step++) {
            $this->highlighted = ($this->highlighted + $direction + $count) % $count;
            if (!$this->visibleOptions[$this->highlighted]->disabled) break;
        }
        $this->scrollToHighlighted();
    }

    private function firstEnabled(): int
    {
        foreach ($this->visibleOptions as $index => $option) if (!$option->disabled) return $index;
        return -1;
    }

    private function scrollToHighlighted(): void
    {
        if ($this->highlighted < 0) return;
        $top = $this->highlighted * 38.0;
        $bottom = $top + 38;
        if ($top < $this->scrollOffset) $this->scrollOffset = $top;
        elseif ($bottom > $this->scrollOffset + $this->popupHeight - 8 - ($this->filter ? 44 : 0)) {
            $this->scrollOffset = max(0, $bottom - $this->popupHeight + 8 + ($this->filter ? 44 : 0));
        }
        ++$this->scrollRevision;
    }

    public function unmount(): void
    {
        ++$this->generation;
        $this->open = false;
    }
}
?>

<template>
    <Column>
        <Pressable :nativeRef="$reference" :enabled="$enabled" on:press="toggle" on:keyDown="key" :captureKeys="$open ? 'Enter,Space,Escape,ArrowDown,ArrowUp,Home,End' : 'Enter,Space,ArrowDown,ArrowUp'" :accessibilityLabel="$label" accessibilityRole="combobox" :accessibilityExpanded="$open" :backgroundColor="$theme->surface" :borderColor="$theme->border" class="trigger" :height="$fieldStyle->height" :borderRadius="$fieldStyle->radius" :paddingLeft="$fieldStyle->paddingHorizontal" :paddingRight="$fieldStyle->paddingHorizontal" :opacity="$enabled ? 1 : 0.5">
            <Row alignItems="center" gap="8">
                <Text flexGrow="1" :textColor="$theme->text" class="value" :fontSize="$fieldStyle->fontSize" :lineHeight="$fieldStyle->fontSize * 1.5" :fontWeight="$fieldStyle->fontWeight" numberOfLines="1">{{ selectedLabel() }}</Text>
                <View p-if="$showClear && $value !== null" width="24" />
                <Icon p-if="$theme->iconFont !== null" name="chevron-down" :font="$theme->iconFont" size="14" :color="$theme->triggerIconColor ?? $theme->muted" /><Text p-else :textColor="$theme->muted" class="arrow">⌄</Text>
            </Row>
        </Pressable>
        <Pressable p-if="$showClear && $value !== null" position="absolute" right="32" top="0" width="24" :height="$fieldStyle->height" alignItems="center" justifyContent="center" :enabled="$enabled" on:press="clear" :accessibilityLabel="uiText('select.clear')"><Icon p-if="$theme->iconFont !== null" name="times" :font="$theme->iconFont" size="14" :color="$theme->clearIconColor ?? $theme->muted" /><Text p-else :textColor="$theme->muted">×</Text></Pressable>
        <Portal p-if="$open" :name="$reference" :host="$host">
            <View widthPercent="100" heightPercent="100">
                <Pressable position="absolute" top="0" bottom="0" left="0" right="0" on:press="close" :accessibilityLabel="uiText('select.closeOptions')" />
                <Column :left="$popupX" :top="$popupY" :width="$popupWidth" :height="$popupHeight" position="absolute" :backgroundColor="$theme->surface" :borderColor="$theme->border" class="popup">
                    <Input p-if="$filter" :nativeRef="$reference.'-search'" :value="$query" on:change="search" on:keyDown="key" captureKeys="Enter,Escape,ArrowDown,ArrowUp" :placeholder="$filterPlaceholder" :accessibilityLabel="uiText('select.searchOption')" :backgroundColor="$theme->surface" :textColor="$theme->text" class="search" />
                    <Text p-if="$visibleOptions === []" :textColor="$theme->muted" class="empty">{{ $emptyMessage }}</Text>
                    <VirtualizedList p-else flexGrow="1" :scrollRequest="$scrollRevision" :scrollTargetOffset="$scrollOffset" estimatedRowHeight="38" prefetch="2" accessibilityRole="list">
                    <Pressable p-for="$option in $visibleOptions" p-index="index" p-key="$option->key" :nativeRef="$reference.'-option-'.$index" :enabled="!$option->disabled" class="option" :backgroundColor="$index === $highlighted ? $theme->highlight : $theme->surface" :opacity="$option->disabled ? 0.5 : 1" on:press="choose($option->value)" on:keyDown="key" :accessibilityLabel="$option->label" accessibilityRole="button" :accessibilitySelected="$option->value === $value">
                        <Text :textColor="$theme->text" class="value">{{ $option->label }}</Text>
                    </Pressable>
                    </VirtualizedList>
                </Column>
            </View>
        </Portal>
    </Column>
</template>

<style scoped>
    .trigger { height: 40px; padding: 0 12px; border-width: 1px; border-radius: 6px; justify-content: center; }
    .trigger:focus { border-color: #2563EB; }
    .value { font-size: 14px; line-height: 21px; color: #0F172A; }
    .arrow { font-size: 16px; color: #64748B; }
    .popup { padding: 4px; border-width: 1px; border-radius: 6px; }
    .search { height: 40px; padding: 8px; margin-bottom: 4px; border-width: 0; font-size: 14px; }
    .empty { padding: 8px; font-size: 13px; }
    .option { height: 38px; padding: 8px; border-radius: 4px; }
</style>
