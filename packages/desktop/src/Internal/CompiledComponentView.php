<?php

declare(strict_types=1);

namespace Pam\Desktop\Internal;

use Pam\Desktop\Component;
use Pam\Desktop\Element;
use Pam\Desktop\Renderable;

final readonly class CompiledComponentView implements Renderable
{
    public function __construct(
        private Component $component,
        private CompiledTemplateNode $template,
    ) {
    }

    public function toElement(): Element
    {
        $slots = $this->component->__pamSlots();
        $template = $this->template;
        // `$props` is only collected for templates that can read it.
        $props = ($template->pamUsesProps ??= $template->mentionsProps())
            ? PamPhpRegistry::publicProps($this->component)
            : [];

        return TemplateRenderer::render(
            $template,
            $this->component,
            [
                ...$slots,
                'props' => $props,
                '__pamInheritedStyles' => $this->component->__pamInheritedStyles(),
            ],
        );
    }
}
