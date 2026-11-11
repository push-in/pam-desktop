<?php

declare(strict_types=1);

namespace Pam\Desktop\Internal;

use Pam\Desktop\Attributes\Action;
use Pam\Desktop\Attributes\Expose;
use Pam\Desktop\Component;
use Pam\Desktop\TemplateRegistry;
use ReflectionClass;
use RuntimeException;

use function count;
use function in_array;
use function is_string;

final class TemplateContractValidator
{
    private function __construct()
    {
    }

    /** @param class-string<Component> $component */
    public static function validate(string $component, CompiledTemplateNode $tree): void
    {
        $errors = self::diagnostics($component, $tree, 1);
        if ($errors !== []) {
            throw $errors[0];
        }
    }

    /**
     * Every contract error of the template (one per attribute), in source
     * order, for `pam-desktop check`; validate() stops at the first. A
     * language 1 template calls any method of its component, so only a
     * missing action is an error there (language 2 needs #[Action]).
     *
     * @param class-string<Component> $component
     * @return list<RuntimeException>
     */
    public static function diagnostics(
        string $component,
        CompiledTemplateNode $tree,
        int $limit = PHP_INT_MAX,
        bool $declaredActions = true,
    ): array {
        $errors = [];
        self::node(new ReflectionClass($component), $tree, $errors, $limit, $declaredActions, $tree);

        return $errors;
    }

    /**
     * @param ReflectionClass<Component> $component
     * @param list<RuntimeException> $errors
     */
    private static function node(
        ReflectionClass $component,
        CompiledTemplateNode $node,
        array &$errors,
        int $limit,
        bool $declaredActions,
        CompiledTemplateNode $root,
    ): void
    {
        if (count($errors) >= $limit) {
            return;
        }
        foreach ($node->attributes as $name => $value) {
            if (
                !str_starts_with($name, '@')
                && !str_starts_with($name, 'on:')
            ) {
                continue;
            }
            if (!is_string($value) || preg_match('/^[A-Za-z_][A-Za-z0-9_]*$/D', $value) !== 1) {
                continue;
            }
            if (!$component->hasMethod($value)) {
                $errors[] = self::error($node, "action {$value} does not exist on {$component->getName()}");
                continue;
            }
            if (!$declaredActions) {
                continue;
            }
            $method = $component->getMethod($value);
            if (
                !$method->isPublic()
                || ($method->getAttributes(Action::class) === []
                    && $method->getAttributes(Expose::class) === [])
            ) {
                $errors[] = self::error(
                    $node,
                    "action {$value} must be public and declare #[Action] (#[Expose] remains compatible)",
                );
            }
        }

        foreach (self::expressions($node) as $expression) {
            $problem = TemplateExpression::problem($expression, $component);
            if ($problem !== null) {
                $errors[] = self::error($node, "template expression: {$problem}", 'PAM2405');
            }
        }

        if (!TemplateRenderer::knownTag($node->name)) {
            $errors[] = self::error($node, "unknown tag <{$node->name}>: no built-in tag, component or registered tag has this name (a package not booted, such as DesktopUI::boot() or Markdown::register()?)");
        }

        $contract = TemplateRegistry::tagContract($node->name);
        if ($contract !== null) {
            $knownProps = [];
            foreach ($contract->props as $prop) {
                $knownProps[$prop->name] = true;
                if ($prop->required && !isset($node->attributes[$prop->name]) && !isset($node->attributes[':'.$prop->name])) {
                    $errors[] = self::error($node, "required prop {$node->name}.{$prop->name} is missing");
                }
            }
            $knownEvents = [];
            foreach ($contract->events as $event) {
                $knownEvents[$event->name] = true;
            }
            foreach (array_keys($node->attributes) as $attribute) {
                if (str_starts_with($attribute, '@')) {
                    $event = substr($attribute, 1);
                    if (!isset($knownEvents[$event])) {
                        $errors[] = self::error($node, "unknown event {$node->name}.{$event}");
                    }
                    continue;
                }
                $prop = ltrim($attribute, ':');
                if (
                    isset($knownProps[$prop])
                    || in_array($prop, ['class', 'key', 'recipe'], true)
                    || str_starts_with($prop, 'p-')
                    || str_starts_with($prop, 'variant:')
                ) {
                    continue;
                }
                $errors[] = self::error($node, "unknown prop {$node->name}.{$prop}");
            }
        }

        if ($contract === null && $node->name !== '' && TemplateRegistry::isCompiledComponent($node->name)) {
            $props = PamPhpRegistry::propNames($node->name);
            foreach (array_keys($node->attributes) as $attribute) {
                $unknown = $props === [] ? null : TemplateRenderer::unknownComponentAttribute($node->name, (string) $attribute, $props);
                if ($unknown !== null) {
                    $errors[] = self::error($node, $unknown);
                }
            }
        } elseif ($contract === null && $node->name !== '') {
            foreach (array_keys($node->attributes) as $attribute) {
                $unknown = TemplateRenderer::unknownBuiltinAttribute($node->name, (string) $attribute);
                if ($unknown !== null) {
                    $errors[] = self::error($node, $unknown);
                }
            }
        }

        if ($contract === null && $node->name !== '') {
            // Classes of a built-in element: a static `class`, and the string
            // literals of a bound `:class` (each is a class list it can take).
            $lists = [];
            $static = $node->attributes['class'] ?? null;
            if (is_string($static) && !str_contains($static, '{{')) {
                $lists[] = $static;
            }
            $bound = $node->attributes[':class'] ?? null;
            if (is_string($bound)) {
                foreach (self::classLiterals($bound) as $literal) {
                    $lists[] = $literal;
                }
            }
            $reported = [];
            foreach ($lists as $classes) {
                foreach (TemplateRenderer::unknownClasses($root, $classes) as $class) {
                    if (!isset($reported[$class])) {
                        $reported[$class] = true;
                        $errors[] = self::error($node, "class {$class} has no style rule in this component, no registered class and no utility");
                    }
                }
            }
        }

        foreach ($node->children as $child) {
            self::node($component, $child, $errors, $limit, $declaredActions, $root);
        }
    }

    /**
     * String literals of a `:class` expression that are its results (class
     * lists): `$m === 'cash' ? 'a b' : 'b'` yields "a b" and "b", not the
     * compared "cash"; concatenations and arguments are skipped.
     *
     * @return list<string>
     */
    private static function classLiterals(string $expression): array
    {
        if (preg_match_all('/\'([^\'\\\\]*)\'|"([^"\\\\]*)"/', $expression, $matches, PREG_SET_ORDER | PREG_OFFSET_CAPTURE) === 0) {
            return [];
        }
        $literals = [];
        foreach ($matches as $match) {
            [$quoted, $offset] = $match[0];
            $before = rtrim(substr($expression, 0, $offset));
            $after = ltrim(substr($expression, $offset + strlen($quoted)));
            // A result: the whole expression or a ternary/null-coalescing
            // branch (`c ? 'a' : 'b'`, `$x ?? 'a'`, grouped in parentheses),
            // never an argument, array item, comparison or concatenation.
            $opens = $before === '' || preg_match('/(\?|:|\?\?|(^|[^\w\])])\()$/', $before) === 1;
            $closes = $after === '' || preg_match('/^(\)|\?|:(?!:))/', $after) === 1;
            if (!$opens || !$closes || str_ends_with($before, '::')) {
                continue;
            }
            $value = isset($match[2]) && $match[2][1] >= 0 ? $match[2][0] : ($match[1][0] ?? '');
            if (str_contains($value, '$') || str_contains($value, '{')) {
                continue;
            }
            $literals[] = $value;
        }

        return $literals;
    }

    /**
     * The PHP expressions of a node: bound props, directives, handler calls
     * and `{{ }}` interpolations of its text and attributes.
     *
     * @return list<string>
     */
    private static function expressions(CompiledTemplateNode $node): array
    {
        $expressions = [];
        foreach (self::interpolations($node->value) as $expression) {
            $expressions[] = $expression;
        }
        foreach ($node->attributes as $name => $value) {
            if (!is_string($value)) {
                continue;
            }
            $name = (string) $name;
            $directive = preg_replace('/^v-/', 'p-', $name);
            if ($directive === 'p-for') {
                if (preg_match('/^\s*(?:\([^)]*\)|\$?[A-Za-z_]\w*)\s+in\s+(.+)$/sD', $value, $match) === 1) {
                    $expressions[] = $match[1];
                }
            } elseif (str_starts_with($name, ':') || in_array($directive, ['p-if', 'p-else-if', 'p-show', 'p-key'], true)) {
                $expressions[] = $value;
            } elseif (str_starts_with($name, '@') || str_starts_with($name, 'on:')) {
                if (str_contains($value, '(')) {
                    $expressions[] = $value;
                }
            } else {
                foreach (self::interpolations($value) as $expression) {
                    $expressions[] = $expression;
                }
            }
        }

        return $expressions;
    }

    /** @return list<string> */
    private static function interpolations(string $text): array
    {
        if (!str_contains($text, '{{') || preg_match_all('/\{\{(.*?)\}\}/s', $text, $matches) === 0) {
            return [];
        }

        return array_values(array_filter(array_map('trim', $matches[1]), static fn (string $e): bool => $e !== ''));
    }

    private static function error(CompiledTemplateNode $node, string $message, string $code = 'PAM2401'): RuntimeException
    {
        return new RuntimeException(
            "{$code} {$message} at {$node->source}:{$node->line}:{$node->column}.",
        );
    }
}
