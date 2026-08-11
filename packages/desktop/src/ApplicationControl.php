<?php

declare(strict_types=1);

namespace Pam\Desktop;

final readonly class ApplicationControl
{
    public function __construct(private Invocation $invocation)
    {
    }

    public function quit(): void
    {
        $this->invocation->effect(WindowEffect::close('main'));
    }
}
