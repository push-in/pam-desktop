<?php

declare(strict_types=1);

namespace Pam\Desktop;

final readonly class Windows
{
    /** @param list<string> $knownWindows */
    public function __construct(
        private Invocation $invocation,
        private string $currentWindowId,
        private array $knownWindows,
    ) {
    }

    public function current(): WindowHandle
    {
        return $this->get($this->currentWindowId);
    }

    public function main(): WindowHandle
    {
        return $this->get('main');
    }

    public function get(string $id): WindowHandle
    {
        if (!in_array($id, $this->knownWindows, true)) {
            throw new \InvalidArgumentException("Window {$id} is not registered.");
        }

        return new WindowHandle($id, $this->invocation);
    }
}
