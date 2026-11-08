<?php

declare(strict_types=1);

use Pam\Desktop\App;
use Pam\Desktop\Component;
use Pam\Desktop\Element;
use Pam\Desktop\Internal\Runtime;
use Pam\Desktop\Signals\Signal;
use Pam\Desktop\Signals\Signals;
use Pam\Desktop\UI\Column;
use Pam\Desktop\UI\Screen;
use Pam\Desktop\UI\Text;

use function Pam\Desktop\signal;

$assert = require __DIR__.'/arch_v2_bootstrap.php';
Runtime::shutdown();

final class SignalRenderingConsumer extends Component
{
    public int $renders = 0;

    public function __construct(private readonly ?Signal $value = null) {}

    public function render(): Element
    {
        $this->renders++;

        return Text::make((string) ($this->value?->get() ?? 'stable'))->toElement();
    }
}

final class SignalRenderingRoot extends Component
{
    public function __construct(public SignalRenderingConsumer $reader, public SignalRenderingConsumer $row) {}

    public function render(): Element
    {
        return Screen::make(Column::make($this->reader, $this->row))->toElement();
    }
}

$value = signal(0);
$unread = signal(false);
$reader = new SignalRenderingConsumer($value);
$row = new SignalRenderingConsumer();
App::run(new SignalRenderingRoot($reader, $row));
$assert($reader->renders === 1 && $row->renders === 1, 'Both components render on mount.');
$value->set(1);
Runtime::render();
$assert($reader->renders === 2 && $row->renders === 1, 'Only the component reading a changed signal renders.');
$unread->set(true);
Runtime::render();
$assert($reader->renders === 2 && $row->renders === 1, 'A resource flag nobody renders leaves every row memoized.');
Signals::batch(static function () use ($value, $unread): void {
    $value->set(2);
    $unread->set(false);
});
Runtime::render();
$assert($reader->renders === 3 && $row->renders === 1, 'Batched signals also preserve unrelated rows.');
Runtime::shutdown();
echo "ARCH_V2_SIGNAL_RENDERING_OK tracked=updated unrelated=memoized batch=memoized\n";
