<?php

declare(strict_types=1);

require_once dirname(__DIR__, 3).'/vendor/autoload.php';

use Pam\Desktop\Internal\TemplateCompiler;
use Pam\Desktop\Internal\TemplateRenderer;
use Pam\Desktop\PropKey;

foreach (['width' => [PropKey::Width, PropKey::WidthPercent], 'height' => [PropKey::Height, PropKey::HeightPercent],
    'minWidth' => [PropKey::MinWidth, PropKey::MinWidthPercent], 'minHeight' => [PropKey::MinHeight, PropKey::MinHeightPercent],
    'maxWidth' => [PropKey::MaxWidth, PropKey::MaxWidthPercent], 'maxHeight' => [PropKey::MaxHeight, PropKey::MaxHeightPercent]] as $attribute => [$point, $percent]) {
    $template = TemplateCompiler::compile('<Column '.$attribute.'="75.5%" />', 'percentage.pam');
    $props = TemplateRenderer::render($template, new stdClass(), [])->toElement()->properties();
    if (isset($props[$point->value]) || ($props[$percent->value] ?? null) !== 75.5) throw new RuntimeException('Static percentage must use native percentage key: '.$attribute);
    $template = TemplateCompiler::compile('<Column :'.$attribute.'="$size" />', 'bound-percentage.pam');
    $scope = new class { public mixed $size = '50%'; };
    foreach (['50%', 120, '25%'] as $size) {
        $scope->size = $size;
        $props = TemplateRenderer::render($template, $scope, [])->toElement()->properties();
        $key = is_string($size) ? $percent : $point;
        if (($props[$key->value] ?? null) != (float) $size) throw new RuntimeException('Bound dimension must switch units: '.$attribute);
        if (isset($props[(is_string($size) ? $point : $percent)->value])) throw new RuntimeException('Previous unit leaked.');
    }
}
echo "Template dimension percentages: six dimensions, static/bound values and unit changes passed\n";
