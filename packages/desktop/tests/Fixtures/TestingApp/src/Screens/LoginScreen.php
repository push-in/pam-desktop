<?php

declare(strict_types=1);

namespace TestingApp\Screens;

use Pam\Desktop\Component;
use Pam\Desktop\Renderable;
use Pam\Desktop\UI\Text;

final class LoginScreen extends Component
{
    public function render(): Renderable
    {
        return Text::make('login');
    }
}
