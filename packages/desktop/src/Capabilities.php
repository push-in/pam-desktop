<?php

declare(strict_types=1);

namespace Pam\Desktop;

use RuntimeException;

final readonly class Capabilities
{
    /**
     * @param list<FileSystemRoot> $filesystemRoots
     */
    private function __construct(
        public array $filesystemRoots,
        public bool $dialogsEnabled,
        public bool $clipboardReadEnabled,
        public bool $clipboardWriteEnabled,
        public bool $notificationsEnabled,
        public bool $dragAndDropEnabled,
    ) {
        $names = [];
        foreach ($filesystemRoots as $root) {
            if (isset($names[$root->name])) {
                throw new RuntimeException("Filesystem root {$root->name} is already authorized.");
            }
            $names[$root->name] = true;
        }
    }

    public static function none(): self
    {
        return new self([], false, false, false, false, false);
    }

    public function filesystem(FileSystemRoot ...$roots): self
    {
        return new self(
            filesystemRoots: array_values(array_merge($this->filesystemRoots, $roots)),
            dialogsEnabled: $this->dialogsEnabled,
            clipboardReadEnabled: $this->clipboardReadEnabled,
            clipboardWriteEnabled: $this->clipboardWriteEnabled,
            notificationsEnabled: $this->notificationsEnabled,
            dragAndDropEnabled: $this->dragAndDropEnabled,
        );
    }

    public function dialogs(bool $enabled = true): self
    {
        return new self(
            $this->filesystemRoots,
            $enabled,
            $this->clipboardReadEnabled,
            $this->clipboardWriteEnabled,
            $this->notificationsEnabled,
            $this->dragAndDropEnabled,
        );
    }

    public function clipboard(bool $read = true, bool $write = true): self
    {
        return new self(
            $this->filesystemRoots,
            $this->dialogsEnabled,
            $read,
            $write,
            $this->notificationsEnabled,
            $this->dragAndDropEnabled,
        );
    }

    public function notifications(bool $enabled = true): self
    {
        return new self(
            $this->filesystemRoots,
            $this->dialogsEnabled,
            $this->clipboardReadEnabled,
            $this->clipboardWriteEnabled,
            $enabled,
            $this->dragAndDropEnabled,
        );
    }

    public function dragAndDrop(bool $enabled = true): self
    {
        return new self(
            $this->filesystemRoots,
            $this->dialogsEnabled,
            $this->clipboardReadEnabled,
            $this->clipboardWriteEnabled,
            $this->notificationsEnabled,
            $enabled,
        );
    }

    /**
     * @return array{
     *     filesystemRoots: list<array{name: string, path: string, access: int}>,
     *     dialogs: bool,
     *     clipboardRead: bool,
     *     clipboardWrite: bool,
     *     notifications: bool,
     *     dragAndDrop: bool
     * }
     */
    public function toArray(): array
    {
        return [
            'filesystemRoots' => array_map(
                static fn (FileSystemRoot $root): array => $root->toArray(),
                $this->filesystemRoots,
            ),
            'dialogs' => $this->dialogsEnabled,
            'clipboardRead' => $this->clipboardReadEnabled,
            'clipboardWrite' => $this->clipboardWriteEnabled,
            'notifications' => $this->notificationsEnabled,
            'dragAndDrop' => $this->dragAndDropEnabled,
        ];
    }
}
