<?php

declare(strict_types=1);

use Pam\Desktop\Internal\TemplateExpression;
use Pam\Desktop\Internal\TemplateExpressionCompiler;

// Generated expression closures must behave exactly like the reference
// closure tree: same values, same types, same diagnostics.
$codegenScope = new class {
    public array $row = ['id' => 'c-1', 'selected' => true, 'dark' => false, 'title' => 'Ana', 'badge' => 'count', 'unread' => 3];
    public ?string $missingValue = null;
    public int $count = 4;
    public object $profile;

    public function __construct()
    {
        $this->profile = (object) ['name' => 'Pam', 'links' => ['site' => 'https://pam.dev']];
    }

    public function glyph(string $name): string
    {
        return 'asset://glyphs/'.$name.'.png';
    }

    public function double(int $value): int
    {
        return $value * 2;
    }

    public function owner(): object
    {
        return $this->profile;
    }

    public function nobody(): ?object
    {
        return null;
    }

    public function viewer(): object
    {
        return new class {
            public function initials(string $name): string
            {
                return mb_substr($name, 0, 1);
            }
        };
    }
};
$codegenExpressions = [
    "['message-row', 'message-row-selected' => \$row['selected'], 'dark' => \$row['selected'] && \$row['dark']]",
    "\$row['dark'] ? '#FFFFFF' : '#F7F6F2'",
    "\$row['badge'] === 'count'",
    "'channel-preview-button-'.\$row['id']",
    "glyph('checkmark')",
    "\$row['missing'] ?? 'fallback'",
    "\$row['missing']['deeper'] ?? \$missingValue ?? 'last'",
    "\$row['missing']",
    "\$undefinedVariable",
    "\$undefinedVariable ?? 7",
    "\$profile->name",
    "\$profile.links['site']",
    "\$profile->unknown ?? 'none'",
    "\$profile->unknown",
    "\$count + 1 * 2 - 3",
    "(\$count + 1) % 3",
    "\$count / 0",
    "\$count % 2.5",
    "-\$count",
    "-\$row",
    "!\$row['selected'] || \$count >= 4",
    "\$count <=> 1",
    "\$count > 3 && \$count < 5 && \$count <= 4",
    "\$count == '4' && \$count != 5",
    "\$row['title'] ?: 'Untitled'",
    "\$missingValue ?: 'empty'",
    "count(\$row) . ' items'",
    "in_array('c-1', \$row, true)",
    "double(\$count)",
    "mb_strtoupper(mb_substr(\$row['title'], 0, 1))",
    "[\$count => 'numeric', 'list', 'nested' => [\$row['unread'], \$row['unread'] + 1]]",
    "[\$row => 'bad key']",
    "\$row . 'x'",
    "\$count + 'x'",
    "1.5 * 2",
    "'a\\'b' . \"c\\n\"",
    "\$row[\$count]",
    "\$row[\$undefinedVariable ?? 'id']",
    "\$row[[]] ?? 'bad index'",
    "\$count ? \$undefinedVariable : 1",
    "true ? 'yes' : 'no'",
    "null ?? false ?? 'x'",
    "owner()->name",
    "owner()->links['site']",
    "owner()['name'] ?? 'not an array'",
    "viewer()->initials(\$row['title'])",
    "owner()->unknown ?? 'none'",
    "owner()->unknown",
    "(\$count > 3 ? \$profile : null)->name",
    "glyph('a') . 'b'",
    "(\$row)['title']",
    "owner()?->name",
    "nobody()?->name",
    "nobody()?->name ?? 'nobody'",
    "nobody()?->initials(\$undefinedVariable)",
    "\$profile?->name",
    "\$nothing?->name ?? 'empty'",
    "(\$count > 3 ? \$profile : null)?->links['site']",
];
foreach ($codegenExpressions as $codegenExpression) {
    $outcomes = [];
    foreach ([true, false] as $generated) {
        TemplateExpression::useGeneratedCode($generated);
        try {
            $value = TemplateExpression::evaluate($codegenExpression, $codegenScope, ['row' => $codegenScope->row, 'local' => 1]);
            $outcomes[] = 'value:'.get_debug_type($value).':'.serialize(is_object($value) ? get_object_vars($value) : $value);
        } catch (Throwable $codegenError) {
            $outcomes[] = 'error:'.$codegenError::class.':'.$codegenError->getMessage();
        }
    }
    $assert(
        $outcomes[0] === $outcomes[1],
        "Generated template expression must match the reference evaluator: {$codegenExpression} ({$outcomes[0]} vs {$outcomes[1]}).",
    );
}
foreach ([true, false] as $generated) {
    TemplateExpression::useGeneratedCode($generated);
    $nullsafe = static fn (string $expression): mixed => TemplateExpression::evaluate($expression, $codegenScope, ['row' => $codegenScope->row, 'nothing' => null]);
    $assert(
        $nullsafe('owner()?->name') === $codegenScope->profile->name
            && $nullsafe("nobody()?->name ?? 'nobody'") === 'nobody'
            && $nullsafe('nobody()?->initials($undefinedVariable)') === null
            && $nullsafe("\$nothing?->name ?? 'empty'") === 'empty',
        '?-> must end the chain with null on a null value, without evaluating the rest.',
    );
}
TemplateExpression::useGeneratedCode(true);
$assert(
    str_starts_with(TemplateExpressionCompiler::closureSource("\$row['id'] ?? ''"), 'static function (array $data, ?object $scope): mixed {'),
    'Template expressions compile to plain PHP closures.',
);
try {
    TemplateExpressionCompiler::closureSource('$a +');
    $assert(false, 'Invalid expressions must not compile.');
} catch (RuntimeException) {
    $assert(true, 'Invalid expressions are rejected by the expression compiler.');
}

// `:class` array literals compile to class-name lists with the same result.
$classListCases = [
    "['message-row', 'message-row-selected' => \$row['selected'], 'dark' => \$row['selected'] && \$row['dark']]",
    "['a', ' ', 'b' => \$row['missing'] ?? false, 'c' => \$row['nope'], '' => true, 'd' => 0]",
    "['5' => true, 'x' => true]",
    "['x' => true, 'x' => false]",
    "[\$row['title'], 'y' => true]",
    "['z' => \$undefinedVariable]",
];
foreach (['base', 'a', 'b', 'c', 'd', 'x', 'y', 'z', 'Ana', 'message-row', 'message-row-selected', 'dark'] as $classListName) {
    \Pam\Desktop\TemplateRegistry::style($classListName, []);
}
foreach ($classListCases as $classListExpression) {
    $classOutcomes = [];
    foreach ([true, false] as $generated) {
        TemplateExpression::useGeneratedCode($generated);
        try {
            $classTemplate = \Pam\Desktop\Internal\TemplateCompiler::compile(
                '<View class=" base " :class="'.$classListExpression.'" />',
                'ClassList.pam',
            );
            $classElement = \Pam\Desktop\Internal\TemplateRenderer::render($classTemplate, $codegenScope, ['row' => $codegenScope->row]);
            $classOutcomes[] = implode(',', $classElement->domClasses());
        } catch (Throwable $classError) {
            $classOutcomes[] = 'error:'.$classError::class.':'.$classError->getMessage();
        }
    }
    $assert(
        $classOutcomes[0] === $classOutcomes[1],
        "Compiled :class lists must match the reference: {$classListExpression} ({$classOutcomes[0]} vs {$classOutcomes[1]}).",
    );
}
TemplateExpression::useGeneratedCode(true);
