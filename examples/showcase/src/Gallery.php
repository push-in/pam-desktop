<?php

declare(strict_types=1);

namespace App;

use Pam\Desktop\Component;
use Pam\Desktop\View;

final class Gallery extends Component
{
    public function render(): View
    {
        return View::make('screens.gallery');
    }

    public function openCommerce(): void
    {
        $this->pushRoute('commerce');
    }

    public function openFinance(): void
    {
        $this->pushRoute('finance');
    }

    public function openChat(): void
    {
        $this->pushRoute('chat');
    }

    public function openField(): void
    {
        $this->pushRoute('field');
    }

    public function openLab(): void
    {
        $this->pushRoute('home');
    }
}
