<?php

declare(strict_types=1);

use Pam\Desktop\Internal\TemplateExpression;

require_once dirname(__DIR__, 3).'/vendor/autoload.php';

foreach ([false, true] as $generated) {
    TemplateExpression::useGeneratedCode($generated);
    $cases = [
        ['min(440, count($rows) * 66)', ['rows' => range(1, 10)], 440],
        ['max([2, 3.5, -1])', [], 3.5],
        ['array_slice($rows, 1, 2)', ['rows' => ['a', 'b', 'c', 'd']], ['b', 'c']],
        ['array_slice($rows, -1, null, true)', ['rows' => ['a', 'b']], [1 => 'b']],
    ];
    foreach ($cases as [$expression, $data, $expected]) {
        if (TemplateExpression::evaluate($expression, null, $data) !== $expected) {
            throw new RuntimeException('Collection expression mismatch: '.$expression);
        }
    }
    foreach (['min()', 'max([1, "2"])', 'array_slice([], "0")', 'array_slice([], 0, null, 1)'] as $expression) {
        try {
            TemplateExpression::evaluate($expression, null, []);
        } catch (RuntimeException) {
            continue;
        }
        throw new RuntimeException('Invalid expression accepted: '.$expression);
    }
}
TemplateExpression::useGeneratedCode(true);
echo "Template numeric bounds and array slices: generated/reference passed\n";
