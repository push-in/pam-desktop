<?php

declare(strict_types=1);
namespace Pam\DesktopUi;

/** A single bounded result page; the control never accumulates previous pages. */
final readonly class AutocompletePage
{
    /** @param list<AutocompleteOption> $options */
    public function __construct(public array $options, public bool $hasMore = false) {}
}
