<?php

declare(strict_types=1);

namespace Pam\Desktop;

use RuntimeException;

final readonly class Capabilities
{
    /**
     * @param list<FileSystemRoot> $filesystemRoots
     * @param list<Database> $databases
     * @param list<HttpOrigin> $httpOrigins
     * @param list<ProcessCommand> $processes
     */
    private function __construct(
        public array $filesystemRoots,
        public bool $dialogsEnabled,
        public bool $clipboardReadEnabled,
        public bool $clipboardWriteEnabled,
        public bool $notificationsEnabled,
        public bool $dragAndDropEnabled,
        public array $databases,
        public bool $systemInformationEnabled,
        public array $httpOrigins,
        public bool $secretsEnabled,
        public array $processes,
        public bool $desktopPortalEnabled,
    ) {
        $names = [];
        foreach ($filesystemRoots as $root) {
            if (isset($names[$root->name])) {
                throw new RuntimeException("Filesystem root {$root->name} is already authorized.");
            }
            $names[$root->name] = true;
        }
        foreach ($databases as $database) {
            if (isset($names[$database->name])) {
                throw new RuntimeException("Native resource {$database->name} is already authorized.");
            }
            $names[$database->name] = true;
        }
        foreach ($httpOrigins as $origin) {
            if (isset($names[$origin->name])) {
                throw new RuntimeException("Native resource {$origin->name} is already authorized.");
            }
            $names[$origin->name] = true;
        }
        foreach ($processes as $process) {
            if (isset($names[$process->name])) {
                throw new RuntimeException("Native resource {$process->name} is already authorized.");
            }
            $names[$process->name] = true;
        }
    }

    public static function none(): self
    {
        return new self([], false, false, false, false, false, [], false, [], false, [], false);
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
            databases: $this->databases,
            systemInformationEnabled: $this->systemInformationEnabled,
            httpOrigins: $this->httpOrigins,
            secretsEnabled: $this->secretsEnabled,
            processes: $this->processes,
            desktopPortalEnabled: $this->desktopPortalEnabled,
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
            $this->databases,
            $this->systemInformationEnabled,
            $this->httpOrigins,
            $this->secretsEnabled,
            $this->processes,
            $this->desktopPortalEnabled,
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
            $this->databases,
            $this->systemInformationEnabled,
            $this->httpOrigins,
            $this->secretsEnabled,
            $this->processes,
            $this->desktopPortalEnabled,
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
            $this->databases,
            $this->systemInformationEnabled,
            $this->httpOrigins,
            $this->secretsEnabled,
            $this->processes,
            $this->desktopPortalEnabled,
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
            $this->databases,
            $this->systemInformationEnabled,
            $this->httpOrigins,
            $this->secretsEnabled,
            $this->processes,
            $this->desktopPortalEnabled,
        );
    }

    public function database(Database ...$databases): self
    {
        return new self(
            $this->filesystemRoots,
            $this->dialogsEnabled,
            $this->clipboardReadEnabled,
            $this->clipboardWriteEnabled,
            $this->notificationsEnabled,
            $this->dragAndDropEnabled,
            array_values(array_merge($this->databases, $databases)),
            $this->systemInformationEnabled,
            $this->httpOrigins,
            $this->secretsEnabled,
            $this->processes,
            $this->desktopPortalEnabled,
        );
    }

    public function systemInformation(bool $enabled = true): self
    {
        return new self(
            $this->filesystemRoots,
            $this->dialogsEnabled,
            $this->clipboardReadEnabled,
            $this->clipboardWriteEnabled,
            $this->notificationsEnabled,
            $this->dragAndDropEnabled,
            $this->databases,
            $enabled,
            $this->httpOrigins,
            $this->secretsEnabled,
            $this->processes,
            $this->desktopPortalEnabled,
        );
    }

    public function http(HttpOrigin ...$origins): self
    {
        return new self(
            $this->filesystemRoots,
            $this->dialogsEnabled,
            $this->clipboardReadEnabled,
            $this->clipboardWriteEnabled,
            $this->notificationsEnabled,
            $this->dragAndDropEnabled,
            $this->databases,
            $this->systemInformationEnabled,
            array_values(array_merge($this->httpOrigins, $origins)),
            $this->secretsEnabled,
            $this->processes,
            $this->desktopPortalEnabled,
        );
    }

    public function secrets(bool $enabled = true): self
    {
        return new self(
            $this->filesystemRoots,
            $this->dialogsEnabled,
            $this->clipboardReadEnabled,
            $this->clipboardWriteEnabled,
            $this->notificationsEnabled,
            $this->dragAndDropEnabled,
            $this->databases,
            $this->systemInformationEnabled,
            $this->httpOrigins,
            $enabled,
            $this->processes,
            $this->desktopPortalEnabled,
        );
    }

    public function process(ProcessCommand ...$commands): self
    {
        return new self(
            $this->filesystemRoots,
            $this->dialogsEnabled,
            $this->clipboardReadEnabled,
            $this->clipboardWriteEnabled,
            $this->notificationsEnabled,
            $this->dragAndDropEnabled,
            $this->databases,
            $this->systemInformationEnabled,
            $this->httpOrigins,
            $this->secretsEnabled,
            array_values(array_merge($this->processes, $commands)),
            $this->desktopPortalEnabled,
        );
    }

    public function desktopPortal(bool $enabled = true): self
    {
        return new self(
            $this->filesystemRoots,
            $this->dialogsEnabled,
            $this->clipboardReadEnabled,
            $this->clipboardWriteEnabled,
            $this->notificationsEnabled,
            $this->dragAndDropEnabled,
            $this->databases,
            $this->systemInformationEnabled,
            $this->httpOrigins,
            $this->secretsEnabled,
            $this->processes,
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
     *     dragAndDrop: bool,
     *     databases: list<array{name: string, path: string, access: int}>,
     *     systemInformation: bool,
     *     httpOrigins: list<array{name: string, origin: string}>,
     *     secrets: bool,
     *     processes: list<array{name: string, executable: string, arguments: list<string>, argumentPolicy: int}>,
     *     desktopPortal: bool
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
            'databases' => array_map(
                static fn (Database $database): array => $database->toArray(),
                $this->databases,
            ),
            'systemInformation' => $this->systemInformationEnabled,
            'httpOrigins' => array_map(
                static fn (HttpOrigin $origin): array => $origin->toArray(),
                $this->httpOrigins,
            ),
            'secrets' => $this->secretsEnabled,
            'processes' => array_map(
                static fn (ProcessCommand $process): array => $process->toArray(),
                $this->processes,
            ),
            'desktopPortal' => $this->desktopPortalEnabled,
        ];
    }
}
