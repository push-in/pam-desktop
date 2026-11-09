<?php

declare(strict_types=1);

namespace Pam\DesktopUi;

use Closure;
use Pam\Desktop\Attributes\State;
use Pam\Desktop\Component;
use Pam\Desktop\I18n\FrameworkText;
use Pam\Desktop\KeyEvent;
use Pam\Desktop\System\ComponentTimer;
use Pam\Desktop\System\ViewRef;
use Pam\Desktop\ViewMeasurement;
use Throwable;

/** Editable, bounded remote suggestions. The provider returns a cancellation closure. */
final class Autocomplete extends Component
{
    use Concerns\LocalizedText;

    #[State(hotReload: false)] public bool $open = false;
    #[State(hotReload: false)] public bool $loading = false;
    #[State(hotReload: false)] public string $error = '';
    /** @var list<AutocompleteOption> */
    #[State(hotReload: false)] public array $options = [];
    #[State(hotReload: false)] public int $highlighted = -1;
    #[State(hotReload: false)] public float $popupX = 0;
    #[State(hotReload: false)] public float $popupY = 0;
    #[State(hotReload: false)] public float $popupWidth = 0;
    #[State(hotReload: false)] public float $popupHeight = 0;
    #[State(hotReload: false)] public float $scrollOffset = 0;
    #[State(hotReload: false)] public int $scrollRevision = 0;
    private ?ComponentTimer $blurTimer = null;
    #[State(hotReload: false)] public int $page = 1;
    #[State(hotReload: false)] public bool $hasMore = false;
    #[State(hotReload: false)] public string $query = '';
    public readonly string $reference;
    public readonly string $filterReference;
    public readonly string $anchorReference;
    public readonly string $filterPlaceholder;
    public readonly string $filterLabel;
    public readonly SelectTheme $theme;
    public readonly AutocompleteInputStyle $inputStyle;
    /** @var array<string,true> */
    #[State(hotReload: false)] public array $failedImages = [];
    private int $generation = 0;
    private ?Closure $cancel = null;
    private ?ComponentTimer $debounce = null;
    /**
     * @param Closure(string):void $changed
     * @param Closure(int|string):void $selected
     * @param Closure(string, Closure(list<AutocompleteOption>|AutocompletePage):void, Closure(Throwable):void):?Closure $suggest
     * @param null|Closure(string, int, Closure(list<AutocompleteOption>|AutocompletePage):void, Closure(Throwable):void):?Closure $pageProvider
     */
    public function __construct(
        #[State] public string $value,
        private readonly Closure $changed,
        private readonly Closure $selected,
        private readonly Closure $suggest,
        #[State] public bool $enabled = true,
        public string $placeholder = '',
        public ?string $label = null,
        public string $host = 'desktop-ui',
        ?SelectTheme $theme = null,
        private readonly int $minimumLength = 2,
        private readonly int $limit = 8,
        private readonly int $debounceMs = 250,
        public bool $showImages = true,
        public bool $searchOnFocus = false,
        private readonly ?Closure $pageProvider = null,
        ?AutocompleteInputStyle $inputStyle = null,
        public readonly bool $filterInPopup = false,
        #[State] public int|string|null $selectedValue = null,
        private readonly ?Closure $cleared = null,
        ?string $filterPlaceholder = null,
    ) {
        if ($minimumLength < 0 || $limit < 1 || $limit > 20 || $debounceMs < 0 || $debounceMs > 2000) {
            throw new \InvalidArgumentException('Invalid autocomplete bounds.');
        }
        $this->label ??= FrameworkText::get('autocomplete.label');
        $this->theme = $theme ?? new SelectTheme();
        $this->inputStyle = $inputStyle ?? new AutocompleteInputStyle();
        $this->reference = 'autocomplete-'.spl_object_id($this);
        $this->filterReference = $this->reference.'-filter';
        $this->anchorReference = $this->filterInPopup ? $this->reference.'-anchor' : $this->reference;
        $this->filterLabel = FrameworkText::get('autocomplete.filter', ['label' => $this->label]);
        $this->filterPlaceholder = $filterPlaceholder ?? $this->filterLabel;
    }
    public function input(string $value): void
    {
        if ($this->filterInPopup) {
            $this->filter($value);
            return;
        }
        if (!$this->enabled) {
            return;
        }
        $this->dismiss();
        $this->value = mb_substr($value, 0, 256);
        ($this->changed)($this->value);
        if (mb_strlen(trim($this->value)) < $this->minimumLength) {
            return;
        }
        $generation = $this->generation;
        $this->debounce = $this->timers()->timeout($this->debounceMs, function () use ($generation): void {
            if ($generation === $this->generation) {
                $this->search();
            }
        });
    }
    public function filter(string $value): void
    {
        if (!$this->filterInPopup || !$this->enabled || !$this->open) {
            return;
        }
        $this->query = mb_substr($value, 0, 120);
        $this->debounce?->cancel();
        $this->debounce = null;
        $this->invalidateRequest();
        $this->options = [];
        $this->loading = false;
        $this->error = '';
        $this->highlighted = -1;
        $this->scrollOffset = 0;
        ($this->changed)($this->query);
        if (mb_strlen(trim($this->query)) < $this->minimumLength) {
            return;
        }
        $generation = $this->generation;
        $this->debounce = $this->timers()->timeout($this->debounceMs, function () use ($generation): void {
            if ($generation === $this->generation) {
                $this->search(1, $this->query);
            }
        });
    }
    private function invalidateRequest(): void
    {
        ++$this->generation;
        $cancel = $this->cancel;
        $this->cancel = null;
        $cancel?->__invoke();
    }
    /** Immediate query also permits explicit retry without a second debounce. */
    public function search(int $page = 1, ?string $query = null): void
    {
        if ($this->filterInPopup && $this->open) {
            $this->debounce?->cancel();
            $this->debounce = null;
            $cancel = $this->cancel;
            $this->cancel = null;
            $cancel?->__invoke();
            ++$this->generation;
            $this->scrollOffset = 0;
            ++$this->scrollRevision;
            $this->options = [];
            $this->loading = false;
            $this->error = '';
            $this->highlighted = -1;
        } else {
            $this->dismiss();
        }
        $query = trim($query ?? $this->value);
        if (!$this->enabled || mb_strlen($query) < $this->minimumLength) {
            return;
        }
        $generation = $this->generation;
        $this->query = $query;
        $this->page = max(1, $page);
        $this->hasMore = false;
        $this->loading = true;
        $this->place($generation);
        try {
            $complete = function (array|AutocompletePage $result) use ($generation): void {
                if ($generation !== $this->generation) {
                    return;
                }
                $options = $result instanceof AutocompletePage ? $result->options : $result;
                $this->hasMore = $result instanceof AutocompletePage && $result->hasMore;
                if (count($options) > $this->limit) {
                    throw new \UnexpectedValueException('Autocomplete provider exceeded its result limit.');
                }
                $keys = [];
                foreach ($options as $option) {
                    if (isset($keys[$option->key])) {
                        throw new \UnexpectedValueException('Invalid autocomplete option.');
                    }
                    $keys[$option->key] = true;
                }
                $this->options = array_values($options);
                $this->loading = false;
                $this->highlighted = $this->selectedIndex();
                $this->place($generation);
            };
            $failed = function (Throwable $error) use ($generation): void {
                if ($generation !== $this->generation) {
                    return;
                }
                $this->loading = false;
                $this->error = FrameworkText::get('list.searchFailed');
                $this->place($generation);
            };
            $cancel = $this->pageProvider !== null
                ? ($this->pageProvider)($query, $this->page, $complete, $failed)
                : ($this->suggest)($query, $complete, $failed);
            if ($generation === $this->generation) {
                $this->cancel = $cancel;
            } else {
                $cancel?->__invoke();
            }
        } catch (Throwable) {
            if ($generation !== $this->generation) {
                return;
            }
            $this->loading = false;
            $this->error = FrameworkText::get('list.searchFailed');
            $this->place($generation);
        }
    }
    private function place(int $generation): void
    {
        ViewRef::of($this->anchorReference)->measureInWindow(function (?ViewMeasurement $anchor) use ($generation): void {
            if ($generation !== $this->generation || $anchor === null) {
                return;
            }
            ViewRef::of($this->host)->measureInWindow(function (?ViewMeasurement $host) use ($generation, $anchor): void {
                if ($generation !== $this->generation || $host === null) {
                    return;
                }
                $this->popupWidth = min(max($anchor->width, $this->theme->popupMinWidth), max(0, $host->width - 8));
                $this->popupX = max(4, min($anchor->windowX - $host->windowX, $host->width - $this->popupWidth - 4));
                $offset = $this->theme->popupOffset;
                $below = $anchor->windowY - $host->windowY + $anchor->height + $offset;
                $above = $anchor->windowY - $host->windowY - $offset;
                $desired = max($this->paginated() ? 86 : 46, min(280, $this->listHeight() + $this->chromeHeight()));
                $useBelow = $host->height - $below - 4 >= $desired || $host->height - $below - 4 >= $above - 4;
                $this->popupHeight = max(0, min($desired, $useBelow ? $host->height - $below - 4 : $above - 4));
                $this->popupY = $useBelow ? $below : $above - $this->popupHeight;
                $this->open = true;
                if ($this->filterInPopup) {
                    ViewRef::of($this->filterReference)->focus();
                }
            });
        });
    }
    public function choose(int|string $value): void
    {
        if (!$this->enabled) {
            return;
        }
        foreach ($this->options as $option) {
            if ($option->value !== $value || $option->disabled) {
                continue;
            }
            $this->dismiss();
            if ($this->filterInPopup) {
                $this->value = $option->label;
                $this->selectedValue = $value;
            }
            ($this->selected)($value);
            return;
        }
    }
    public function key(KeyEvent|string $payload): void
    {
        $event = $payload instanceof KeyEvent ? $payload : KeyEvent::fromPayload($payload);
        if ($event->control || $event->meta || $event->alt || !$this->enabled) {
            return;
        }
        if ($event->key === 'Escape' || $event->key === 'Tab') {
            $this->dismiss();
            if ($this->filterInPopup && $event->key === 'Escape') {
                ViewRef::of($this->filterReference)->blur();
            }
            return;
        }
        if ($event->key === 'Enter') {
            $option = $this->options[$this->highlighted] ?? null;
            if ($option !== null) {
                $this->choose($option->value);
            }
            return;
        }
        if (!in_array($event->key, ['ArrowDown', 'ArrowUp'], true)) {
            return;
        }
        $count = count($this->options);
        for ($step = 0; $step < $count; $step++) {
            $this->highlighted = ($this->highlighted + ($event->key === 'ArrowUp' ? -1 : 1) + $count) % $count;
            if (!$this->options[$this->highlighted]->disabled) {
                break;
            }
        }
        $top = $this->highlighted * (float) $this->rowHeight();
        if ($top < $this->scrollOffset) {
            $this->scrollOffset = $top;
        } elseif ($top + $this->rowHeight() > $this->scrollOffset + $this->popupHeight - $this->chromeHeight()) {
            $this->scrollOffset = max(0, $top + $this->rowHeight() + $this->chromeHeight() - $this->popupHeight);
        }
        ++$this->scrollRevision;
    }
    public function nextPage(): void { if (!$this->loading && $this->hasMore) $this->search($this->page + 1, $this->query); }
    public function previousPage(): void { if (!$this->loading && $this->page > 1) $this->search($this->page - 1, $this->query); }
    public function paginated(): bool { return $this->pageProvider !== null; }
    /** A single complete page needs no pager, matching a plain select list. */
    public function showsPager(): bool { return $this->paginated() && ($this->page > 1 || $this->hasMore); }
    public function clearable(): bool { return $this->cleared !== null && $this->enabled && $this->value !== ''; }
    public function popupBorderColor(): int { return $this->theme->popupBorder ?? $this->theme->border; }
    public function filterBorderColor(): int { return $this->theme->filterBorder ?? $this->theme->border; }
    public function clearIconColor(): int { return $this->theme->clearIconColor ?? $this->theme->muted; }
    public function triggerIconColor(): int { return $this->theme->triggerIconColor ?? $this->theme->muted; }
    public function isSelected(AutocompleteOption $option): bool { return $this->selectedValue !== null && $option->value === $this->selectedValue; }
    public function optionSurface(int $index, AutocompleteOption $option): int
    {
        if ($this->isSelected($option)) {
            return $this->theme->selectedSurface ?? $this->theme->highlight;
        }
        return $index === $this->highlighted ? $this->theme->highlight : 0x00000000;
    }
    /** The theme placeholder colour, else the field text at 45% (the hosts' dimmed default). */
    public function placeholderColor(): int
    {
        return $this->theme->placeholderColor ?? ((0x73 << 24) | ($this->theme->text & 0xFFFFFF));
    }
    public function optionColor(AutocompleteOption $option): int
    {
        return $this->isSelected($option) ? ($this->theme->selectedText ?? $this->optionTextColor()) : $this->optionTextColor();
    }
    /** Select mode keeps a 2px gap between options (36px unless the theme sets `optionHeight`), like a PrimeVue list. */
    public function optionHeight(): int|float { return $this->filterInPopup ? $this->rowHeight() - 2 : $this->rowHeight(); }
    private function listHeight(): int|float { return max(0, count($this->options) * $this->rowHeight() - ($this->filterInPopup && $this->options !== [] ? 2 : 0)); }
    private function chromeHeight(): int { return 10 + ($this->showsPager() ? 40 : 0) + ($this->filterInPopup ? 54 : 0); }
    private function selectedIndex(): int
    {
        foreach ($this->options as $index => $option) {
            if ($this->isSelected($option) && !$option->disabled) {
                return $index;
            }
        }
        return -1;
    }
    public function clear(): void
    {
        if (!$this->clearable() || $this->cleared === null) {
            return;
        }
        $this->dismiss();
        $this->value = '';
        $this->selectedValue = null;
        ($this->cleared)();
    }
    public function inputSurfaceColor(): int { return $this->theme->inputSurface ?? $this->theme->surface; }
    public function popupSurfaceColor(): int { return $this->theme->popupSurface ?? $this->theme->surface; }
    public function filterSurfaceColor(): int { return $this->theme->filterSurface ?? $this->theme->inputSurface ?? $this->theme->surface; }
    public function optionTextColor(): int { return $this->theme->optionText ?? $this->theme->text; }
    public function triggerAccessibilityLabel(): string { return FrameworkText::get($this->open ? 'autocomplete.close' : 'autocomplete.open', ['label' => mb_strtolower((string) $this->label)]); }

    public function imageFailed(string $key): void
    {
        foreach ($this->options as $option) {
            if ($option->key === $key) {
                $this->failedImages[$key] = true;
                return;
            }
        }
    }
    public function blur(): void
    {
        $this->blurTimer?->cancel();
        $this->blurTimer = $this->timers()->timeout(120, fn () => $this->dismiss());
    }
    public function filterFocus(): void
    {
        $this->blurTimer?->cancel();
        $this->blurTimer = null;
    }
    public function toggle(): void
    {
        if (!$this->enabled) {
            return;
        }
        if ($this->open) {
            $this->dismiss();
            return;
        }
        $this->search(1, '');
    }
    public function rowHeight(): int|float { return $this->showImages ? 64 : ($this->theme->optionHeight === null ? 38 : $this->theme->optionHeight + ($this->filterInPopup ? 2 : 0)); }

    public function focus(): void
    {
        $this->blurTimer?->cancel();
        $this->blurTimer = null;
        if ($this->searchOnFocus) $this->search(1, '');
    }
    public function dismiss(): void
    {
        ++$this->generation;
        $this->blurTimer?->cancel();
        $this->blurTimer = null;
        $this->scrollOffset = 0;
        ++$this->scrollRevision;
        $this->debounce?->cancel();
        $this->debounce = null;
        $cancel = $this->cancel;
        $this->cancel = null;
        $cancel?->__invoke();
        $this->open = $this->loading = false;
        $this->options = [];
        $this->failedImages = [];
        $this->error = '';
        $this->highlighted = -1;
    }
    public function paused(): void
    {
        $this->dismiss();
    }
    public function unmount(): void
    {
        $this->dismiss();
    }
}
?>
<template>
    <Column>
        <Input p-if="!$filterInPopup" :nativeRef="$reference" :value="$value" :enabled="$enabled" :placeholder="$placeholder" :placeholderColor="placeholderColor()" :accessibilityLabel="$label" accessibilityRole="combobox" :accessibilityExpanded="$open" on:change="input" on:blur="blur" on:focus="focus" on:keyDown="key" captureKeys="Enter,Escape,ArrowDown,ArrowUp" :backgroundColor="inputSurfaceColor()" :textColor="$theme->text" :borderColor="$theme->border" class="input" :height="$inputStyle->height" :fontSize="$inputStyle->fontSize" :fontWeight="$inputStyle->fontWeight" :paddingLeft="$inputStyle->paddingHorizontal" :paddingRight="$inputStyle->paddingHorizontal" :borderRadius="$inputStyle->radius" />
        <Row p-else :nativeRef="$anchorReference" class="select-input" alignItems="center" :backgroundColor="inputSurfaceColor()" :borderColor="$open && $theme->focusBorder !== null ? $theme->focusBorder : $theme->border" :shadowColor="$open && $theme->focusRing !== null ? $theme->focusRing : 0" :shadowSpreadRadius="$open && $theme->focusRing !== null ? $theme->focusRingWidth : 0">
            <Input :nativeRef="$reference" :value="$value" :enabled="$enabled" :editable="false" :placeholder="$placeholder" :placeholderColor="placeholderColor()" :accessibilityLabel="$label" accessibilityRole="combobox" :accessibilityExpanded="$open" on:blur="blur" on:focus="focus" on:keyDown="key" captureKeys="Enter,Escape,ArrowDown,ArrowUp" backgroundColor="0" :textColor="$theme->text" :borderColor="0" class="select-value" :height="$inputStyle->height - 2" :fontSize="$inputStyle->fontSize" :fontWeight="$inputStyle->fontWeight" :paddingLeft="$inputStyle->paddingHorizontal" :paddingRight="$inputStyle->paddingHorizontal" />
            <Pressable p-if="clearable()" class="select-clear" :accessibilityLabel="uiText('autocomplete.clear', ['label' => mb_strtolower($label)])" accessibilityRole="button" on:press="clear"><Icon p-if="$theme->iconFont !== null" name="times" :font="$theme->iconFont" size="14" :color="clearIconColor()" /><Text p-else :textColor="clearIconColor()" fontSize="16">×</Text></Pressable>
            <Pressable class="select-trigger" :enabled="$enabled" :accessibilityLabel="triggerAccessibilityLabel()" accessibilityRole="button" on:press="toggle"><Icon p-if="$theme->iconFont !== null" name="chevron-down" :font="$theme->iconFont" size="14" :color="triggerIconColor()" /><Text p-else :textColor="triggerIconColor()" fontSize="14">⌄</Text></Pressable>
        </Row>
        <Portal p-if="$open" :name="$reference" :host="$host">
            <View widthPercent="100" heightPercent="100" pointerEvents="box-none">
                <Column :left="$popupX" :top="$popupY" :width="$popupWidth" :height="$popupHeight" position="absolute" :backgroundColor="popupSurfaceColor()" :borderColor="popupBorderColor()" :elevation="$theme->popupShadowColor === null ? $theme->popupElevation : 0" :shadowColor="$theme->popupShadowColor ?? 0" :shadowOffsetY="$theme->popupShadowOffsetY" :shadowBlurRadius="$theme->popupShadowBlur" class="popup">
                    <View p-if="$filterInPopup" class="filter-header">
                        <Input :nativeRef="$filterReference" :value="$query" :enabled="$enabled" :placeholder="$filterPlaceholder" :accessibilityLabel="$filterLabel" accessibilityRole="search" on:change="filter" on:blur="blur" on:focus="filterFocus" on:keyDown="key" captureKeys="Enter,Escape,ArrowDown,ArrowUp" :backgroundColor="filterSurfaceColor()" :textColor="$theme->text" :borderColor="filterBorderColor()" class="filter" :fontSize="$inputStyle->fontSize" :paddingLeft="$inputStyle->paddingHorizontal" :paddingRight="$theme->iconFont !== null ? 40 : $inputStyle->paddingHorizontal" />
                        <Icon p-if="$theme->iconFont !== null" class="filter-icon" name="search" :font="$theme->iconFont" size="16" :color="$theme->muted" />
                    </View>
                    <Text p-if="$loading" :textColor="$theme->muted" class="message">Buscando...</Text>
                    <Pressable p-else-if="$error !== ''" on:press="search" class="message"><Text :textColor="$theme->muted">{{ $error }}</Text></Pressable>
                    <Text p-else-if="$options === []" :textColor="$theme->muted" class="message">{{ uiText('list.empty') }}</Text>
                    <VirtualizedList p-else flexGrow="1" class="list" :estimatedRowHeight="rowHeight()" prefetch="1" :scrollRequest="$scrollRevision" :scrollTargetOffset="$scrollOffset">
                        <Pressable p-for="$option in $options" p-index="index" p-key="$option->key" :enabled="!$option->disabled" :class="$filterInPopup ? 'option option-select' : 'option'" :height="optionHeight()" :marginBottom="rowHeight() - optionHeight()" :backgroundColor="optionSurface($index, $option)" on:press="choose($option->value)" :accessibilityLabel="$option->label" :accessibilitySelected="isSelected($option)"><Row gap="12" alignItems="center"><Image p-if="$showImages && $option->image !== '' && !($failedImages[$option->key] ?? false)" on:error="imageFailed($option->key)" :source="$option->image" width="48" height="48" resizeMode="cover" /><View p-else-if="$showImages" width="48" height="48" justifyContent="center"><Text :textColor="$theme->muted" fontSize="10">Sem foto</Text></View><Column gap="4" flexGrow="1"><Text :textColor="optionColor($option)" :fontFamily="$theme->fontFamily" :fontSize="$theme->optionFontSize" :fontWeight="$theme->optionFontWeight" numberOfLines="1">{{ $option->label }}</Text><Text p-if="$option->detail !== ''" :textColor="$theme->muted" :fontFamily="$theme->fontFamily" fontSize="12" numberOfLines="1">{{ $option->detail }}</Text></Column></Row></Pressable>
                    </VirtualizedList>
                    <Row p-if="showsPager()" class="pager" height="40" alignItems="center" gap="8"><Button class="page-button" :backgroundColor="popupSurfaceColor()" :textColor="$theme->text" :label="uiText('list.previous')" :enabled="!$loading && $page > 1" on:press="previousPage" /><Text :textColor="$theme->muted" flexGrow="1" textAlign="center">{{ $page }}</Text><Button class="page-button" :backgroundColor="popupSurfaceColor()" :textColor="$theme->text" :label="uiText('list.next')" :enabled="!$loading && $hasMore" on:press="nextPage" /></Row>
                </Column>
            </View>
        </Portal>
    </Column>
</template>
<style scoped>
.input { height:40px; padding:8px 12px; border-width:1px; border-radius:6px; font-size:14px; }
.select-input { height:50px; border-width:1px; border-radius:8px; }
.select-input:focus-within { border-color:#2563EB; }
.select-value { flex-grow:1; border-width:0; background-color:rgba(0,0,0,0); }
.select-clear { width:16px; height:16px; align-items:center; justify-content:center; }
.select-trigger { width:40px; height:40px; align-items:center; justify-content:center; }
.popup { border-width:1px; border-radius:6px; overflow:hidden; }
.list { padding:4px; }
.filter-header { padding:8px 16px 4px 16px; }
.filter-icon { position:absolute; right:28px; top:21px; }
.option { padding:8px; border-radius:4px; }
.option-select { padding:0 12px; justify-content:center; }
.filter { height:42px; padding:8px 12px; border-width:1px; border-radius:6px; }
.pager { border-top-width:1px; border-top-color:rgba(255,255,255,0.08); }
.page-button { height:32px; padding:0 8px; border-width:0; border-radius:4px; background-color:rgba(0,0,0,0); font-size:12px; }
.message { padding:8px; font-size:13px; }
</style>
