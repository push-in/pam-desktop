<?php

declare(strict_types=1);

// Standalone runs use the project autoloader; tests/run.php brings its own.
if (!class_exists(\Pam\Desktop\Protocol::class)) {
    require_once dirname(__DIR__, 3).'/vendor/autoload.php';
}

use Pam\Desktop\Internal\TemplateExpression;

$scope = new class {
    public int $calls = 0;
    public function tick(): int { return ++$this->calls; }
    public function fail(): never { throw new RuntimeException('Unselected expression was evaluated.'); }
};
foreach ([false, true] as $generated) {
    TemplateExpression::useGeneratedCode($generated);
    $scope->calls = 0;
    foreach ([
        ['true ? 4 : fail()', 4],
        ['false ? fail() : 5', 5],
        ['7 ?: fail()', 7],
        ['false ?: 8', 8],
        ['true || fail()', true],
        ['false && fail()', false],
        ['9 ?? fail()', 9],
        ['null ?? 10', 10],
        ['$absent ?? 11', 11],
        ['$row->id === null ? "Sem vínculo" : $names[$row->id]', 'Sem vínculo'],
        ['false || (true && (false ? fail() : tick()))', true],
    ] as [$expression, $expected]) {
        $actual = TemplateExpression::evaluate($expression, $scope, ['row' => (object) ['id' => null], 'names' => []]);
        if ($actual !== $expected) throw new RuntimeException('Short circuit mismatch: '.$expression);
    }
    if ($scope->calls !== 1) throw new RuntimeException('Chosen branch must execute exactly once.');
}
TemplateExpression::useGeneratedCode(true);
echo "Template short-circuit guards: ternary, shorthand, coalesce, logical operators and null indexed branch passed\n";
