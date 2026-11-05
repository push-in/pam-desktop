<?php

declare(strict_types=1);

namespace TestingApp\Screens;

use Pam\Desktop\Component;
use Pam\Desktop\Composables\ColorScheme;
use Pam\Desktop\Composables\KeyboardState;
use Pam\Desktop\Composables\LocaleState;
use Pam\Desktop\Composables\SafeAreaInsets;
use Pam\Desktop\Renderable;
use Pam\Desktop\Signals\Signal;
use Pam\Desktop\UI\Text;

use function Pam\Desktop\onBlur;
use function Pam\Desktop\onFocus;
use function Pam\Desktop\onMounted;
use function Pam\Desktop\onUnmounted;
use function Pam\Desktop\signal;
use function Pam\Desktop\useColorScheme;
use function Pam\Desktop\useKeyboard;
use function Pam\Desktop\useLocale;
use function Pam\Desktop\useSafeAreaInsets;

final class ChatScreen extends Component
{
    public string $id = '';
    public int $unread = 0;
    public ?Signal $draft = null;
    public ?ColorScheme $scheme = null;
    public ?SafeAreaInsets $insets = null;
    public ?KeyboardState $keyboard = null;
    public ?LocaleState $locale = null;

    public function __construct(public string $title = 'Chat')
    {
    }

    public function setup(string $id = '0', int $unread = 0): void
    {
        $this->id = $id;
        $this->unread = $unread;
        $this->draft = signal('');
        $this->scheme = useColorScheme();
        $this->insets = useSafeAreaInsets();
        $this->keyboard = useKeyboard();
        $this->locale = useLocale();
        onMounted(fn () => Log::$entries[] = "chat{$id}:mounted");
        onUnmounted(fn () => Log::$entries[] = "chat{$id}:unmounted");
        onFocus(fn () => Log::$entries[] = "chat{$id}:focus");
        onBlur(fn () => Log::$entries[] = "chat{$id}:blur");
        Log::$entries[] = "chat{$id}:setup";
    }

    public function render(): Renderable
    {
        return Text::make(sprintf(
            '%s %s draft=%s scheme=%s bottom=%s keyboard=%s locale=%s',
            $this->title,
            $this->id,
            $this->draft?->value,
            $this->scheme?->value,
            $this->insets?->bottom,
            $this->keyboard?->height,
            $this->locale?->value,
        ));
    }
}
