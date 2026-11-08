<?php

declare(strict_types=1);

use Pam\Desktop\Internal\TemplateExpression;

require_once dirname(__DIR__, 3).'/vendor/autoload.php';

enum QualifiedExpressionKind: int
{
    case Order = 1;
    case Tab = 2;
    public const DEFAULT = self::Order;
}

// Fully qualified and qualified names reach enum cases and class constants in
// template expressions (`\App\Models\EntryKind::Order`), in both evaluators.
foreach ([true, false] as $generated) {
    TemplateExpression::useGeneratedCode($generated);
    $same = TemplateExpression::evaluate('\QualifiedExpressionKind::Tab === $kind', null, ['kind' => QualifiedExpressionKind::Tab]);
    $value = TemplateExpression::evaluate('\QualifiedExpressionKind::Order->value', null, []);
    $constant = TemplateExpression::evaluate('\Pam\Desktop\Protocol::DESKTOP_BASE', null, []);
    if ($same !== true || $value !== 1 || $constant !== \Pam\Desktop\Protocol::DESKTOP_BASE) {
        throw new RuntimeException('Qualified enum cases and constants must evaluate in template expressions.');
    }
}
TemplateExpression::useGeneratedCode(true);
echo "Qualified enum cases and class constants in template expressions: passed\n";
