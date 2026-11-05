<?php

declare(strict_types=1);

namespace TestingApp\Screens;

use Pam\Desktop\Component;
use Pam\Desktop\Renderable;
use Pam\Desktop\UI\Text;

use function Pam\Desktop\onBlur;
use function Pam\Desktop\onFocus;

final class HomeScreen extends Component
{
    public function setup(): void
    {
        onFocus(static fn () => Log::$entries[] = 'home:focus');
        onBlur(static fn () => Log::$entries[] = 'home:blur');
    }

    public function render(): Renderable
    {
        return Text::make('home');
    }
}
