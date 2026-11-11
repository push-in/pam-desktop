<?php

declare(strict_types=1);

namespace Pam\DesktopUi;

use Pam\Desktop\Attributes\State;
use Pam\Desktop\Component;
use Pam\Desktop\System\ComponentTimer;

/**
 * PrimeVue-style toast stack rendered in a Portal over `host`: severity
 * colours, position, auto-dismiss after `life`, close button and a bounded
 * queue (the oldest message leaves when `limit` is exceeded). Timers stop
 * while the screen is paused and the remaining time resumes with it.
 * The translucent surface blurs what is behind it by `backdropBlur` px
 * (CSS `backdrop-filter: blur()`, 1.5px in PrimeVue Aura; 0 turns it off).
 *
 * For one stack per window, drawn above modals and kept across navigation,
 * use the service instead: `Toast::service()->success('Saved')`.
 */
final class Toast extends Component
{
    use Concerns\LocalizedText;

    /** @var list<ToastMessage> */
    #[State(hotReload: false)] public array $messages = [];
    public readonly string $reference;
    private int $next = 1;
    /** @var array<int, ComponentTimer> */
    private array $timers = [];
    /** @var array<int, float> remaining ms per message while paused */
    private array $remaining = [];
    /** @var array<int, float> deadline (ms since epoch) per message */
    private array $deadlines = [];
    private bool $suspended = false;
    private ?\Closure $emptied = null;
    /** The theme drawn by the current render (a Closure theme is called again on every render). */
    public private(set) ToastTheme $theme;
    /** @var ToastTheme|\Closure(): ToastTheme */
    private ToastTheme|\Closure $themeSource;

    public function __construct(
        public string $host = 'desktop-ui',
        public readonly ToastPosition $position = ToastPosition::TopRight,
        public readonly int $limit = 5,
        public readonly float $width = 400,
        public readonly float $offset = 20,
        public readonly string $fontFamily = 'system-ui',
        public readonly float $backdropBlur = 1.5,
        ToastTheme|\Closure $theme = new ToastTheme(),
    ) {
        $this->themeSource = $theme;
        $this->theme = ToastTheme::resolve($theme);
        if ($limit < 1 || $limit > 20 || $width < 160 || $width > 800 || $offset < 0 || $offset > 200 || !($backdropBlur >= 0 && $backdropBlur <= 50)) {
            throw new \InvalidArgumentException('Invalid toast geometry.');
        }
        $this->reference = 'toast-'.spl_object_id($this);
    }

    public function rendering(): void
    {
        $this->theme = ToastTheme::resolve($this->themeSource);
    }

    /**
     * The window toast service (PrimeVue ToastService): one stack per
     * window, above modals and sheets, kept while the app navigates.
     */
    public static function service(): ToastService
    {
        return ToastService::instance();
    }

    /** Queues a message and returns its id (for remove()). */
    public function add(ToastSeverity $severity, string $summary, string $detail = '', ?int $life = 3000, bool $closable = true): int
    {
        $message = new ToastMessage($this->next++, $severity, $summary, $detail, $life, $closable);
        $messages = [...$this->messages, $message];
        while (count($messages) > $this->limit) {
            $this->forget(array_shift($messages)->id);
        }
        $this->messages = $messages;
        if ($life !== null) {
            $this->schedule($message->id, (float) $life);
        }

        return $message->id;
    }

    public function success(string $summary, string $detail = '', ?int $life = 3000): int { return $this->add(ToastSeverity::Success, $summary, $detail, $life); }
    public function info(string $summary, string $detail = '', ?int $life = 3000): int { return $this->add(ToastSeverity::Info, $summary, $detail, $life); }
    public function warn(string $summary, string $detail = '', ?int $life = 3000): int { return $this->add(ToastSeverity::Warn, $summary, $detail, $life); }
    public function error(string $summary, string $detail = '', ?int $life = 3000): int { return $this->add(ToastSeverity::Error, $summary, $detail, $life); }

    public function remove(int $id): void
    {
        $this->forget($id);
        $this->messages = array_values(array_filter($this->messages, static fn (ToastMessage $message): bool => $message->id !== $id));
        if ($this->messages === [] && $this->emptied !== null) ($this->emptied)();
    }

    public function clear(): void
    {
        foreach ($this->messages as $message) {
            $this->forget($message->id);
        }
        $this->messages = [];
        if ($this->emptied !== null) ($this->emptied)();
    }

    /** @internal ToastService: told when the last message leaves. */
    public function onEmpty(?\Closure $listener): void
    {
        $this->emptied = $listener;
    }

    private function schedule(int $id, float $milliseconds): void
    {
        if ($this->suspended) {
            $this->remaining[$id] = $milliseconds;
            return;
        }
        $this->deadlines[$id] = microtime(true) * 1000 + $milliseconds;
        $this->timers[$id] = $this->timers()->timeout((int) max(1, round($milliseconds)), fn () => $this->remove($id));
    }

    private function forget(int $id): void
    {
        ($this->timers[$id] ?? null)?->cancel();
        unset($this->timers[$id], $this->remaining[$id], $this->deadlines[$id]);
    }

    public function paused(): void
    {
        $this->suspended = true;
        $now = microtime(true) * 1000;
        foreach ($this->timers as $id => $timer) {
            $timer->cancel();
            $this->remaining[$id] = max(1.0, ($this->deadlines[$id] ?? $now) - $now);
        }
        $this->timers = [];
        $this->deadlines = [];
    }

    public function resumed(): void
    {
        $this->suspended = false;
        $remaining = $this->remaining;
        $this->remaining = [];
        foreach ($remaining as $id => $milliseconds) {
            $this->schedule($id, $milliseconds);
        }
    }

    public function icon(ToastMessage $message): string { return $message->severity->icon(); }
    public function accent(ToastMessage $message): int { return $this->theme->accent($message->severity); }
    public function surface(ToastMessage $message): int { return $this->theme->surface($message->severity); }
    public function border(ToastMessage $message): int { return $this->theme->border($message->severity); }
    public function edge(ToastMessage $message): int { return $this->theme->edge($message->severity); }
    public function summaryColor(ToastMessage $message): int { return $this->theme->summary($message->severity); }
    public function detailColor(ToastMessage $message): int { return $this->theme->detail($message->severity); }
    public function alignment(): string { return $this->position->alignment(); }
    public function vertical(): string { return $this->position->vertical(); }

    public function unmount(): void
    {
        foreach ($this->timers as $timer) {
            $timer->cancel();
        }
        $this->timers = $this->remaining = $this->deadlines = [];
    }
}
?>
<template>
    <Portal p-if="$messages !== []" :name="$reference" :host="$host">
        <Column widthPercent="100" heightPercent="100" pointerEvents="box-none" :padding="$offset" :alignItems="alignment()" :justifyContent="vertical()">
            <Column :width="$width" gap="12" pointerEvents="box-none">
                <Row p-for="$message in $messages" p-key="$message->id" class="message" :backgroundColor="surface($message)" :borderColor="border($message)" :borderLeftWidth="$theme->edgeWidth > 0 ? $theme->edgeWidth : 1" :borderLeftColor="$theme->edgeWidth > 0 ? edge($message) : border($message)" :borderRadius="$theme->radius" :paddingVertical="$theme->paddingVertical" :paddingHorizontal="$theme->paddingHorizontal" :shadowColor="$theme->shadowColor" :shadowOffsetY="$theme->shadowOffsetY" :shadowBlurRadius="$theme->shadowBlur" :backdropBlurRadius="$backdropBlur" accessibilityRole="alert" :accessibilityLabel="$message->summary">
                    <Icon :name="icon($message)" font="Lucide" :size="$theme->iconSize" :color="accent($message)" />
                    <Column flexGrow="1" flexShrink="1" minWidth="0" :gap="$theme->detailGap">
                        <Text :fontFamily="$fontFamily" :fontSize="$theme->summaryFontSize" :lineHeight="$theme->summaryLineHeight" :fontWeight="$theme->summaryFontWeight" :textColor="summaryColor($message)">{{ $message->summary }}</Text>
                        <Text p-if="$message->detail !== ''" :fontFamily="$fontFamily" :fontSize="$theme->detailFontSize" :lineHeight="$theme->detailLineHeight" :fontWeight="$theme->detailFontWeight" :textColor="detailColor($message)">{{ $message->detail }}</Text>
                    </Column>
                    <Pressable p-if="$message->closable" class="close" :marginLeft="$theme->closeOffsetX" :marginRight="-$theme->closeOffsetX" :marginTop="$theme->closeOffsetY" :marginBottom="-$theme->closeOffsetY" accessibilityRole="button" :accessibilityLabel="uiText('toast.close')" on:press="remove($message->id)"><Icon name="x" font="Lucide" :size="$theme->closeIconSize" :color="accent($message)" /></Pressable>
                </Row>
            </Column>
        </Column>
    </Portal>
</template>
<style scoped>
    .message { gap: 8px; align-items: flex-start; border-width: 1px; }
    .close { width: 28px; height: 28px; border-radius: 14px; align-items: center; justify-content: center; }
</style>
