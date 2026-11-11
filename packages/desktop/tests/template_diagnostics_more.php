<?php

declare(strict_types=1);
// Standalone runs use the project autoloader; tests/run.php brings its own.
if (!class_exists(\Pam\Desktop\Protocol::class)) {
    require_once dirname(__DIR__, 3).'/vendor/autoload.php';
}

use Pam\Desktop\Internal\{TemplateCompiler,TemplateRenderer};
use Pam\Desktop\PropKey;

$fail = static fn (string $message) => throw new RuntimeException($message);

// Typos on elements that used to pass silently.
TemplateRenderer::unknownBuiltinAttribute('Pressable', 'on:pres') === 'unknown event Pressable.on:pres, did you mean on:press?' || $fail('An unknown event is reported.');
TemplateRenderer::unknownBuiltinAttribute('Pressable', 'on:press') === null || $fail('Known events pass.');
str_starts_with((string) TemplateRenderer::unknownBuiltinAttribute('Input', 'placeholdr'), 'unknown attribute Input.placeholdr, did you mean placeholder') || $fail('Input attributes are checked.');
TemplateRenderer::unknownBuiltinAttribute('Input', ':value') === null && TemplateRenderer::unknownBuiltinAttribute('ScrollView', 'horizontal') === null || $fail('Tag-specific attributes pass.');
TemplateRenderer::unknownBuiltinAttribute('ScrollView', 'horizontl') !== null || $fail('ScrollView attributes are checked.');

// Component tags: props, root attributes and events.
$props = ['size' => true, 'min' => true];
TemplateRenderer::unknownComponentAttribute('SplitView', ':sizee', $props) === 'unknown prop SplitView.sizee, did you mean size?' || $fail('An unknown component prop is reported.');
TemplateRenderer::unknownComponentAttribute('SplitView', 'flexGrow', $props) === null && TemplateRenderer::unknownComponentAttribute('SplitView', 'on:resize', $props) === null || $fail('Root attributes and events pass.');

// A bound null leaves a colour unset instead of failing.
$element = TemplateRenderer::render(TemplateCompiler::compile('<Text :textColor="null">x</Text>', 'null-colour.pam'), new stdClass(), [])->toElement();
!array_key_exists(PropKey::TextColor->value, $element->properties()) || $fail('A null colour is unset.');
// $event in a handler expression is typed for the kinds that have a payload class.
$scope = new class () {
    public mixed $got = null;
    public function pick(int $row, mixed $event): void
    {
        $this->got = [$row, $event];
    }
};
$pressable = TemplateRenderer::render(TemplateCompiler::compile('<Pressable on:press="pick(3, $event)"><Text>x</Text></Pressable>', 'typed-event.pam'), $scope, [])->toElement();
$pressable->events()[\Pam\Desktop\EventKind::Press->value](\Pam\Desktop\Internal\Wire::map(['x' => 12.5, 'y' => 4, 'pageX' => 12.5, 'pageY' => 4, 'timestamp' => 1, 'pointerId' => 0]));
$scope->got[0] === 3 && $scope->got[1] instanceof \Pam\Desktop\PressEvent && $scope->got[1]->x === 12.5 || $fail('$event is a PressEvent: '.get_debug_type($scope->got[1] ?? null));
echo "Template diagnostics: unknown events, element and component attributes, null colours: passed\n";
