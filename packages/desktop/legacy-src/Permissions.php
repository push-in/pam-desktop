<?php

declare(strict_types=1);

namespace Pam\Desktop;

final class Permissions
{
    private Capabilities $capabilities;

    public function __construct()
    {
        $this->capabilities = Capabilities::none();
    }

    public function filesystem(string $name, string $path, bool $read = true, bool $write = false): self
    {
        $root = match (true) {
            $read && $write => FileSystemRoot::readWrite($name, $path),
            $write => FileSystemRoot::write($name, $path),
            default => FileSystemRoot::read($name, $path),
        };
        $this->capabilities = $this->capabilities->filesystem($root);

        return $this;
    }

    public function dialogs(): self
    {
        $this->capabilities = $this->capabilities->dialogs();

        return $this;
    }

    public function clipboard(bool $read = true, bool $write = true): self
    {
        $this->capabilities = $this->capabilities->clipboard($read, $write);

        return $this;
    }

    public function notifications(): self
    {
        $this->capabilities = $this->capabilities->notifications();

        return $this;
    }

    public function dragAndDrop(): self
    {
        $this->capabilities = $this->capabilities->dragAndDrop();

        return $this;
    }

    public function database(string $name, string $path, bool $write = true): self
    {
        $database = $write
            ? Database::readWrite($name, $path)
            : Database::read($name, $path);
        $this->capabilities = $this->capabilities->database($database);

        return $this;
    }

    public function http(string $name, string $origin): self
    {
        $this->capabilities = $this->capabilities->http(HttpOrigin::allow($name, $origin));

        return $this;
    }

    public function process(ProcessCommand $command): self
    {
        $this->capabilities = $this->capabilities->process($command);

        return $this;
    }

    public function secrets(): self
    {
        $this->capabilities = $this->capabilities->secrets();

        return $this;
    }

    public function systemInformation(): self
    {
        $this->capabilities = $this->capabilities->systemInformation();

        return $this;
    }

    public function desktopPortal(): self
    {
        $this->capabilities = $this->capabilities->desktopPortal();

        return $this;
    }

    public function capabilities(): Capabilities
    {
        return $this->capabilities;
    }
}
