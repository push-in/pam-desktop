<?php

declare(strict_types=1);

namespace App\Components;

use Pam\Desktop\Attributes\Action;
use Pam\Desktop\Attributes\Prop;
use Pam\Desktop\Attributes\State;
use Pam\Desktop\Attributes\Tag;
use Pam\Desktop\Component;

#[Tag('LanguageTwoCard')]
final class LanguageTwoCard extends Component
{
    public function __construct(
        #[Prop(required: true)]
        public readonly string $title,
    ) {
    }

    #[State]
    public bool $selected = false;

    #[Action]
    public function toggle(): void
    {
        $this->selected = !$this->selected;
    }
}
