<?php

declare(strict_types=1);

namespace App;

/** A row of the example list; its methods are called from the template. */
enum OrderStatus: string
{
    case Open = 'open';
    case Paid = 'paid';

    public function label(): string
    {
        return $this === self::Open ? 'Open' : 'Paid';
    }
}

final readonly class Order
{
    public function __construct(public int $id, public OrderStatus $status, public int $cents) {}

    public function total(): string
    {
        return number_format($this->cents / 100, 2);
    }
}
