<?php

declare(strict_types=1);

namespace Pam\DesktopUi;

use Closure;
use Pam\Desktop\AccessibilityCheckedState;
use Pam\Desktop\Attributes\State;
use Pam\Desktop\Component;
use Pam\Desktop\I18n\FrameworkText;
use Pam\Desktop\KeyEvent;
use Pam\Desktop\System\ComponentTimer;
use Pam\Desktop\System\ViewRef;
use Pam\Desktop\ViewMeasurement;
use Throwable;

/** Searchable multi-select with bounded, replace-on-page remote results. */
final class MultiSelect extends Component
{
    use Concerns\LocalizedText;

    #[State(hotReload: false)] public bool $open = false;
    #[State(hotReload: false)] public bool $loading = false;
    #[State(hotReload: false)] public string $error = '';
    #[State(hotReload: false)] public string $query = '';
    /** @var list<AutocompleteOption> */
    #[State] public array $options = [];
    /** @var list<AutocompleteOption> */
    #[State] public array $selectedOptions = [];
    #[State(hotReload: false)] public int $highlighted = -1;
    #[State(hotReload: false)] public float $popupX = 0;
    #[State(hotReload: false)] public float $popupY = 0;
    #[State(hotReload: false)] public float $popupWidth = 0;
    #[State(hotReload: false)] public float $popupHeight = 0;
    #[State(hotReload: false)] public int $scrollRevision = 0;
    #[State(hotReload: false)] public float $scrollOffset = 0;
    #[State(hotReload: false)] public int $page = 1;
    #[State(hotReload: false)] public bool $hasMore = false;

    public readonly string $reference;
    public readonly SelectTheme $theme;

    private int $generation = 0;
    private ?Closure $cancel = null;
    private ?ComponentTimer $debounce = null;
    private ?ComponentTimer $blurTimer = null;

    /**
     * @param list<AutocompleteOption> $selectedOptions
     * @param Closure(list<AutocompleteOption>):void $changed
     * @param Closure(string,int,Closure(list<AutocompleteOption>|AutocompletePage):void,Closure(Throwable):void):?Closure $pageProvider
     */
    public function __construct(
        array $selectedOptions,
        private readonly Closure $changed,
        private readonly Closure $pageProvider,
        #[State] public bool $enabled = true,
        public ?string $placeholder = null,
        public ?string $label = null,
        public string $host = 'desktop-ui',
        ?SelectTheme $theme = null,
        private readonly int $limit = 8,
        private readonly int $minimumLength = 0,
        private readonly int $debounceMs = 250,
        public bool $searchOnOpen = true,
    ) {
        if ($limit < 1 || $limit > 20 || $minimumLength < 0 || $debounceMs < 0 || $debounceMs > 2000) {
            throw new \InvalidArgumentException('Invalid multi-select bounds.');
        }
        $this->placeholder ??= FrameworkText::get('multiSelect.placeholder');
        $this->label ??= FrameworkText::get('multiSelect.label');
        $this->theme = $theme ?? new SelectTheme();
        $this->reference = 'multi-select-'.spl_object_id($this);
        $this->replaceSelection($selectedOptions, notify: false);
        ViewRef::of($host);
    }

    public function toggle(): void
    {
        if (!$this->enabled) return;
        if ($this->open) {
            $this->dismiss();
            return;
        }
        $generation = ++$this->generation;
        ViewRef::of($this->reference)->measureInWindow(function (?ViewMeasurement $anchor) use ($generation): void {
            if ($generation !== $this->generation || $anchor === null || !$this->enabled) return;
            ViewRef::of($this->host)->measureInWindow(function (?ViewMeasurement $host) use ($generation, $anchor): void {
                if ($generation !== $this->generation || $host === null || !$this->enabled) return;
                $this->place($anchor, $host);
                $this->open = true;
                ViewRef::of($this->reference.'-search')->focus();
                if ($this->searchOnOpen) $this->search(1);
            });
        });
    }

    public function place(ViewMeasurement $anchor, ViewMeasurement $host): void
    {
        $this->popupWidth = min($anchor->width, max(0, $host->width - 8));
        $this->popupX = max(4, min($anchor->windowX - $host->windowX, $host->width - $this->popupWidth - 4));
        $below = $anchor->windowY - $host->windowY + $anchor->height + 4;
        $above = $anchor->windowY - $host->windowY - 4;
        $desired = min(320, 48 + min($this->limit, count($this->options)) * 42 + 44);
        $useBelow = $host->height - $below - 4 >= $desired || $host->height - $below - 4 >= $above - 4;
        $this->popupHeight = max(0, min($desired, $useBelow ? $host->height - $below - 4 : $above - 4));
        $this->popupY = $useBelow ? $below : $above - $this->popupHeight;
    }

    public function input(string $value): void
    {
        if (!$this->enabled || !$this->open) return;
        $this->query = mb_substr($value, 0, 120);
        $this->debounce?->cancel();
        $this->debounce = null;
        $this->invalidateRequest();
        $this->options = [];
        $this->loading = false;
        $this->error = '';
        if (mb_strlen(trim($this->query)) < $this->minimumLength) {
            return;
        }
        $generation = $this->generation;
        $this->debounce = $this->timers()->timeout($this->debounceMs, function () use ($generation): void {
            if ($generation === $this->generation) $this->search(1);
        });
    }

    public function search(int $page = 1): void
    {
        if (!$this->enabled || !$this->open || mb_strlen(trim($this->query)) < $this->minimumLength) return;
        $this->debounce?->cancel();
        $this->debounce = null;
        $this->cancelRequest();
        $generation = ++$this->generation;
        $this->page = max(1, $page);
        $this->hasMore = false;
        $this->loading = true;
        $this->error = '';

        $complete = function (array|AutocompletePage $result) use ($generation): void {
            if ($generation !== $this->generation) return;
            $options = $result instanceof AutocompletePage ? $result->options : $result;
            if (count($options) > $this->limit) throw new \UnexpectedValueException('Multi-select provider exceeded its result limit.');
            $keys = [];
            foreach ($options as $option) {
                if (!$option instanceof AutocompleteOption || isset($keys[$option->key])) throw new \UnexpectedValueException('Invalid multi-select option.');
                $keys[$option->key] = true;
            }
            $this->options = array_values($options);
            $this->hasMore = $result instanceof AutocompletePage && $result->hasMore;
            $this->loading = false;
            $this->highlighted = $this->firstEnabled();
            $this->placeCurrent();
        };
        $failed = function (Throwable $error) use ($generation): void {
            if ($generation !== $this->generation) return;
            $this->loading = false;
            $this->error = FrameworkText::get('list.searchFailed');
            $this->placeCurrent();
        };

        try {
            $cancel = ($this->pageProvider)(trim($this->query), $this->page, $complete, $failed);
            if ($generation === $this->generation) $this->cancel = $cancel;
            else $cancel?->__invoke();
        } catch (Throwable $error) {
            $failed($error);
        }
    }

    private function placeCurrent(): void
    {
        $generation = $this->generation;
        ViewRef::of($this->reference)->measureInWindow(function (?ViewMeasurement $anchor) use ($generation): void {
            if ($generation !== $this->generation || $anchor === null) return;
            ViewRef::of($this->host)->measureInWindow(function (?ViewMeasurement $host) use ($generation, $anchor): void {
                if ($generation === $this->generation && $host !== null) $this->place($anchor, $host);
            });
        });
    }

    public function toggleOption(int|string $value): void
    {
        if (!$this->enabled || !$this->open) return;
        foreach ($this->options as $option) {
            if ($option->value !== $value || $option->disabled) continue;
            $selected = $this->selectedOptions;
            $found = false;
            foreach ($selected as $index => $item) {
                if ($item->key !== $option->key) continue;
                array_splice($selected, $index, 1);
                $found = true;
                break;
            }
            if (!$found) $selected[] = $option;
            $this->replaceSelection($selected);
            return;
        }
    }

    /** @param list<AutocompleteOption> $options */
    public function replaceSelection(array $options, bool $notify = true): void
    {
        $unique = [];
        foreach ($options as $option) {
            if (!$option instanceof AutocompleteOption) throw new \InvalidArgumentException('Multi-select values must be AutocompleteOption instances.');
            $unique[$option->key] = $option;
        }
        $this->selectedOptions = array_values($unique);
        if ($notify) ($this->changed)($this->selectedOptions);
    }

    public function selected(int|string $value): bool
    {
        foreach ($this->selectedOptions as $option) if ($option->value === $value) return true;
        return false;
    }

    public function checkedState(int|string $value): int
    {
        return ($this->selected($value) ? AccessibilityCheckedState::Checked : AccessibilityCheckedState::Unchecked)->value;
    }

    public function selectedLabel(): string
    {
        if ($this->selectedOptions === []) return $this->placeholder;
        return implode(', ', array_map(static fn (AutocompleteOption $option): string => $option->label, $this->selectedOptions));
    }

    public function nextPage(): void { if (!$this->loading && $this->hasMore) $this->search($this->page + 1); }
    public function previousPage(): void { if (!$this->loading && $this->page > 1) $this->search($this->page - 1); }

    public function key(KeyEvent|string $payload): void
    {
        $event = $payload instanceof KeyEvent ? $payload : KeyEvent::fromPayload($payload);
        if ($event->control || $event->meta || $event->alt || !$this->enabled || !$this->open) return;
        if ($event->key === 'Escape' || $event->key === 'Tab') { $this->dismiss(); return; }
        if ($event->key === 'Enter') {
            $option = $this->options[$this->highlighted] ?? null;
            if ($option !== null) $this->toggleOption($option->value);
            return;
        }
        if (!in_array($event->key, ['ArrowDown', 'ArrowUp'], true) || $this->options === []) return;
        $count = count($this->options);
        for ($step = 0; $step < $count; ++$step) {
            $this->highlighted = ($this->highlighted + ($event->key === 'ArrowUp' ? -1 : 1) + $count) % $count;
            if (!$this->options[$this->highlighted]->disabled) break;
        }
        $top = $this->highlighted * 42.0;
        if ($top < $this->scrollOffset) $this->scrollOffset = $top;
        elseif ($top + 42 > $this->scrollOffset + $this->popupHeight - 96) $this->scrollOffset = max(0, $top + 42 - $this->popupHeight + 96);
        ++$this->scrollRevision;
    }

    public function blur(): void
    {
        $this->blurTimer?->cancel();
        $this->blurTimer = $this->timers()->timeout(120, fn () => $this->dismiss());
    }

    public function focus(): void
    {
        $this->blurTimer?->cancel();
        $this->blurTimer = null;
    }

    public function dismiss(): void
    {
        ++$this->generation;
        $this->blurTimer?->cancel();
        $this->blurTimer = null;
        $this->debounce?->cancel();
        $this->debounce = null;
        $this->cancelRequest();
        $this->open = $this->loading = false;
        $this->options = [];
        $this->error = '';
        $this->highlighted = -1;
        $this->scrollOffset = 0;
        ++$this->scrollRevision;
    }

    private function invalidateRequest(): void
    {
        ++$this->generation;
        $this->cancelRequest();
    }

    private function cancelRequest(): void
    {
        $cancel = $this->cancel;
        $this->cancel = null;
        $cancel?->__invoke();
    }

    private function firstEnabled(): int
    {
        foreach ($this->options as $index => $option) if (!$option->disabled) return $index;
        return -1;
    }

    public function rowHeight(): int { return 42; }
    public function paginated(): bool { return true; }
    public function paused(): void { $this->dismiss(); }
    public function unmount(): void { $this->dismiss(); }
}
?>
<template>
    <Column>
        <Pressable :nativeRef="$reference" :enabled="$enabled" on:press="toggle" on:keyDown="key" captureKeys="Enter,Space,Escape,ArrowDown,ArrowUp" :accessibilityLabel="$selectedOptions === [] ? $label : selectedLabel()" accessibilityRole="combobox" :accessibilityExpanded="$open" :backgroundColor="$theme->surface" :borderColor="$theme->border" class="trigger" :opacity="$enabled ? 1 : 0.5">
            <Row alignItems="center" gap="8"><Text flexGrow="1" numberOfLines="1" :textColor="$theme->text">{{ selectedLabel() }}</Text><Text :textColor="$theme->muted">⌄</Text></Row>
        </Pressable>
        <Portal p-if="$open" :name="$reference" :host="$host">
            <View widthPercent="100" heightPercent="100">
                <Pressable position="absolute" top="0" bottom="0" left="0" right="0" on:press="dismiss" :accessibilityLabel="uiText('select.closeOptions')" />
                <Column :left="$popupX" :top="$popupY" :width="$popupWidth" :height="$popupHeight" position="absolute" :backgroundColor="$theme->surface" :borderColor="$theme->border" class="popup">
                    <Input :nativeRef="$reference.'-search'" :value="$query" :enabled="$enabled" on:change="input" on:blur="blur" on:focus="focus" on:keyDown="key" captureKeys="Enter,Escape,ArrowDown,ArrowUp" :placeholder="uiText('multiSelect.search')" :accessibilityLabel="uiText('multiSelect.search')" :backgroundColor="$theme->surface" :textColor="$theme->text" :borderColor="$theme->border" class="search" />
                    <Text p-if="$loading" :textColor="$theme->muted" class="message">Buscando...</Text>
                    <Pressable p-else-if="$error !== ''" on:press="search" class="message"><Text :textColor="$theme->muted">{{ $error }}</Text></Pressable>
                    <Text p-else-if="$options === []" :textColor="$theme->muted" class="message">{{ uiText('list.empty') }}</Text>
                    <VirtualizedList p-else flexGrow="1" :estimatedRowHeight="rowHeight()" prefetch="1" :scrollRequest="$scrollRevision" :scrollTargetOffset="$scrollOffset">
                        <Pressable p-for="$option in $options" p-index="index" p-key="$option->key" :nativeRef="$reference.'-option-'.$index" :enabled="!$option->disabled" class="option" :height="rowHeight()" :backgroundColor="$index === $highlighted ? $theme->highlight : $theme->surface" :opacity="$option->disabled ? 0.5 : 1" on:press="toggleOption($option->value)" on:keyDown="key" :accessibilityLabel="$option->label" accessibilityRole="checkbox" :accessibilityCheckedState="checkedState($option->value)">
                            <Row alignItems="center" gap="10"><View width="18" height="18" borderWidth="1" borderRadius="4" :borderColor="selected($option->value) ? $theme->highlight : $theme->muted" :backgroundColor="selected($option->value) ? $theme->highlight : $theme->surface" justifyContent="center" alignItems="center"><Text p-if="selected($option->value)" :textColor="$theme->text" fontSize="12">✓</Text></View><Text flexGrow="1" numberOfLines="1" :textColor="$theme->text">{{ $option->label }}</Text><Text p-if="selected($option->value)" :textColor="$theme->highlight">{{ uiText('multiSelect.selected') }}</Text></Row>
                        </Pressable>
                    </VirtualizedList>
                    <Row height="40" alignItems="center" gap="8"><Button :backgroundColor="$theme->surface" :textColor="$theme->text" :label="uiText('list.previous')" :enabled="!$loading && $page > 1" on:press="previousPage" /><Text :textColor="$theme->muted" flexGrow="1" textAlign="center">{{ $page }}</Text><Button :backgroundColor="$theme->surface" :textColor="$theme->text" :label="uiText('list.next')" :enabled="!$loading && $hasMore" on:press="nextPage" /></Row>
                    <Row justifyContent="end"><Button label="Concluir seleção" :backgroundColor="$theme->highlight" :textColor="$theme->text" on:press="dismiss" /></Row>
                </Column>
            </View>
        </Portal>
    </Column>
</template>
<style scoped>
.trigger { height:50px; padding:10px 14px; border-width:1px; border-radius:8px; }
.popup { padding:6px; border-width:1px; border-radius:8px; }
.search { height:40px; padding:8px 10px; border-width:1px; border-radius:6px; }
.message { padding:10px; font-size:13px; }
.option { padding:8px; border-radius:6px; }
</style>
