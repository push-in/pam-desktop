<?php

declare(strict_types=1);

// Standalone runs use the project autoloader; tests/run.php brings its own.
if (!class_exists(\Pam\Desktop\Protocol::class)) {
    require_once dirname(__DIR__, 3).'/vendor/autoload.php';
}

use Pam\Desktop\Internal\TemplateExpression;

enum ExpressionValueStatus: string
{
    case Paid = 'paid';
    public function label(string $suffix = ''): string { return 'Pago'.$suffix; }
    public static function make(): self { return self::Paid; }
}

final class ExpressionValueItem
{
    public function __construct(private string $name) {}
    public function label(): string { return strtoupper($this->name); }
    public function child(): self { return new self($this->name.'!'); }
    private function secret(): string { return 'secret'; }
}

$scope = new class {
    public function suffix(): string { return '.'; }
};
$data = [
    'order' => ['status' => ExpressionValueStatus::Paid],
    'item' => new ExpressionValueItem('caixa'),
    'callback' => static fn (): string => 'x',
    'nothing' => null,
];
foreach ([false, true] as $generated) {
    TemplateExpression::useGeneratedCode($generated);
    foreach ([
        ['$order[\'status\']->label()', 'Pago'],
        ['$order[\'status\']->label(suffix())', 'Pago.'],
        ['$item->label()', 'CAIXA'],
        ['$item->child()->label() . "?"', 'CAIXA!?'],
        ['$item->label() === "CAIXA" ? 1 : 2', 1],
        ['$nothing->label() ?? "-"', '-'],
        ['$absent->label() ?? "-"', '-'],
    ] as [$expression, $expected]) {
        $actual = TemplateExpression::evaluate($expression, $scope, $data);
        if ($actual !== $expected) throw new RuntimeException("Value method mismatch ({$generated}): {$expression} gave ".var_export($actual, true));
    }
    foreach ([
        '$item->secret()',
        '$item->__construct("x")',
        '$order[\'status\']->make()',
        '$callback->call($item)',
        '$item->missing()',
        '$nothing->label()',
        '"text"->length()',
    ] as $expression) {
        try {
            TemplateExpression::evaluate($expression, $scope, $data);
        } catch (RuntimeException|ParseError) {
            continue;
        }
        throw new RuntimeException("Refused value method was called ({$generated}): {$expression}");
    }
}
TemplateExpression::useGeneratedCode(true);
echo "Template value methods: public instance methods only, chained, inside ?? and refused for private, static, magic and closures passed\n";
