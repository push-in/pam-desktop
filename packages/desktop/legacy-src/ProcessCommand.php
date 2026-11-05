<?php

declare(strict_types=1);

namespace Pam\Desktop;

use InvalidArgumentException;

final readonly class ProcessCommand
{
    /** @param list<string> $arguments */
    private function __construct(
        public string $name,
        public string $executable,
        public array $arguments,
        public ProcessArgumentPolicy $argumentPolicy,
    ) {
        Identifier::assert($name, 'The process command identifier');
        if ($executable === '' || str_contains($executable, "\0") || str_starts_with($executable, '/') || preg_match('~(^|[\\/])\.\.([\\/]|$)~', $executable) === 1) {
            throw new InvalidArgumentException('Process executables must use a safe project-relative path.');
        }
        if (count($arguments) > 32) {
            throw new InvalidArgumentException('Process commands cannot declare more than 32 fixed arguments.');
        }
        foreach ($arguments as $argument) {
            if (strlen($argument) > 1_024 || str_contains($argument, "\0")) {
                throw new InvalidArgumentException('A fixed process argument is invalid.');
            }
        }
    }

    public static function executable(string $name, string $path): self
    {
        return new self($name, $path, [], ProcessArgumentPolicy::Fixed);
    }

    public function arguments(string ...$arguments): self
    {
        return new self($this->name, $this->executable, array_values($arguments), $this->argumentPolicy);
    }

    public function allowArguments(bool $enabled = true): self
    {
        return new self($this->name, $this->executable, $this->arguments, $enabled ? ProcessArgumentPolicy::Append : ProcessArgumentPolicy::Fixed);
    }

    /** @return array{name: string, executable: string, arguments: list<string>, argumentPolicy: int} */
    public function toArray(): array
    {
        return ['name' => $this->name, 'executable' => $this->executable, 'arguments' => $this->arguments, 'argumentPolicy' => $this->argumentPolicy->value];
    }
}
