<?php

declare(strict_types=1);

namespace Pam\Desktop;

use Closure;

final class Desktop
{
    /** @var list<string> */
    private array $windows = ['main'];

    /** @var array<class-string<DesktopWindow>, string> */
    private array $windowClasses = [];

    public function __construct(private readonly Application $application)
    {
    }

    public function permissions(Closure $configure): self
    {
        $permissions = new Permissions();
        $configure($permissions);
        $this->application->capabilities($permissions->capabilities());

        return $this;
    }

    public function window(string $id, Window $window): self
    {
        $this->application->window($id, $window);
        $this->windows[] = $id;

        return $this;
    }

    /** @param class-string<DesktopWindow> $class */
    public function windowClass(string $class, string $id, Window $window): self
    {
        $this->window($id, $window);
        $this->windowClasses[$class] = $id;

        return $this;
    }

    public function shell(Shell $shell): self
    {
        $this->application->shell($shell);

        return $this;
    }

    public function workstation(Workstation $workstation): self
    {
        $this->application->workstation($workstation);

        return $this;
    }

    public function timeout(int $milliseconds): self
    {
        $this->application->commandTimeout($milliseconds);

        return $this;
    }

    public function workers(int $workers): self
    {
        $this->application->parallelWorkers($workers);

        return $this;
    }

    /** @return list<string> */
    public function windowIds(): array
    {
        return $this->windows;
    }

    /** @return array<class-string<DesktopWindow>, string> */
    public function windowClasses(): array
    {
        return $this->windowClasses;
    }
}
