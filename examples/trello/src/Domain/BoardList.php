<?php

declare(strict_types=1);

namespace App\Domain;

final readonly class BoardList
{
    /** @param list<string> $cardIds */
    public function __construct(public string $id, public string $title, public array $cardIds = [])
    {
    }

    /** @param list<string> $cardIds */
    public function withCards(array $cardIds): self
    {
        return new self($this->id, $this->title, $cardIds);
    }
}
