<?php

declare(strict_types=1);

namespace Pam\Desktop;

use InvalidArgumentException;

final readonly class PluginPermissions
{
    /** @param list<string> $filesystemRoots */
    public function __construct(
        public array $filesystemRoots = [],
        public bool $network = false,
        public bool $shell = false,
        public bool $devices = false,
    ) {
        if (count($filesystemRoots) > 16) {
            throw new InvalidArgumentException('A plugin can authorize at most 16 filesystem roots.');
        }
        if (count(array_unique($filesystemRoots)) !== count($filesystemRoots)) {
            throw new InvalidArgumentException('Plugin filesystem roots cannot be duplicated.');
        }
        foreach ($filesystemRoots as $root) {
            RustPlugin::assertProjectPath($root);
        }
    }

    /** @return array{filesystemRoots: list<string>, network: bool, shell: bool, devices: bool} */
    public function toArray(): array
    {
        return [
            'filesystemRoots' => $this->filesystemRoots,
            'network' => $this->network,
            'shell' => $this->shell,
            'devices' => $this->devices,
        ];
    }
}
