<?php

declare(strict_types=1);
require_once dirname(__DIR__, 3).'/vendor/autoload.php';
use Pam\Desktop\{DragPayload,EventKind,PropKey};
use Pam\Desktop\Internal\{TemplateCompiler,TemplateRenderer,Wire};
use Pam\Desktop\UI\{DragSource};

$scope = new class () {
    public ?DragPayload $received = null;
    public function drop(DragPayload $payload): void
    {
        $this->received = $payload;
    }
};
$template = TemplateCompiler::compile('<DropTarget dropType="1" on:drop="drop"><DragSource dragType="1" dragData="order-7"><Text>Order</Text></DragSource></DropTarget>', 'typed-drag.pam');
$element = TemplateRenderer::render($template, $scope, [])->toElement();
if (($element->properties()[PropKey::DropType->value] ?? null) !== 1) {
    throw new RuntimeException('Tag drop type missing.');
}
$handlers = $element->events();
$handlers[EventKind::Drop->value](Wire::map(['type' => 1,'data' => 'order-7']));
if ($scope->received?->data !== 'order-7' || $scope->received->type !== 1) {
    throw new RuntimeException('Native drop event must be decoded to DragPayload.');
}
foreach ([0,-1,65536,'invalid'] as $type) {
    try {
        DragSource::make()->payload($type, 'id');
        throw new RuntimeException('Invalid type accepted.');
    } catch (InvalidArgumentException|TypeError) {
    }
}
try {
    DragSource::make()->payload(1, str_repeat('x', 513));
    throw new RuntimeException('Oversized data accepted.');
} catch (InvalidArgumentException) {
}
echo "Typed native drag tags, payload decoding and limits passed.\n";
