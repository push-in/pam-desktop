<?php

declare(strict_types=1);

require dirname(__DIR__).'/vendor/autoload.php';

/**
 * Keep this representation deliberately language-neutral and line-oriented.
 * Reviewing an intentional additive API change should produce a small snapshot
 * diff, while removals and signature changes fail the compatibility suite.
 */
function parameterSignature(ReflectionParameter $parameter): string
{
    $signature = '';
    if ($parameter->isPassedByReference()) {
        $signature .= '&';
    }
    if ($parameter->isVariadic()) {
        $signature .= '...';
    }

    $signature .= '$'.$parameter->getName();
    if ($parameter->hasType()) {
        $signature .= ':'.$parameter->getType();
    }
    if ($parameter->isDefaultValueAvailable()) {
        $default = $parameter->isDefaultValueConstant()
            ? $parameter->getDefaultValueConstantName()
            : json_encode(
                $parameter->getDefaultValue(),
                JSON_THROW_ON_ERROR | JSON_UNESCAPED_SLASHES,
            );
        $signature .= '='.$default;
    }

    return $signature;
}

function methodSignature(ReflectionClass $class, ReflectionMethod $method): string
{
    $parameters = array_map(
        parameterSignature(...),
        $method->getParameters(),
    );
    $returnType = $method->hasReturnType() ? (string) $method->getReturnType() : '-';

    return sprintf(
        'method %s::%s %s(%s):%s',
        $class->getName(),
        $method->getName(),
        $method->isStatic() ? 'static ' : '',
        implode(',', $parameters),
        $returnType,
    );
}

function constantSignature(
    ReflectionClass $class,
    ReflectionClassConstant $constant,
): string {
    return sprintf(
        'constant %s::%s=%s',
        $class->getName(),
        $constant->getName(),
        json_encode(
            $constant->getValue(),
            JSON_THROW_ON_ERROR | JSON_UNESCAPED_SLASHES,
        ),
    );
}

function propertySignature(ReflectionClass $class, ReflectionProperty $property): string
{
    return sprintf(
        'property %s::$%s%s:%s',
        $class->getName(),
        $property->getName(),
        $property->isReadOnly() ? ' readonly' : '',
        $property->hasType() ? (string) $property->getType() : '-',
    );
}

$sourceFiles = glob(dirname(__DIR__).'/src/*.php');
if ($sourceFiles === false) {
    throw new RuntimeException('Could not discover the public PHP API sources.');
}

$surface = [];
foreach ($sourceFiles as $sourceFile) {
    $name = pathinfo($sourceFile, PATHINFO_FILENAME);
    $className = 'Pam\\Desktop\\'.$name;
    if (
        !class_exists($className)
        && !interface_exists($className)
        && !enum_exists($className)
    ) {
        throw new RuntimeException("Could not reflect public symbol {$className}.");
    }

    $class = new ReflectionClass($className);
    $kind = $class->isEnum()
        ? 'enum'
        : ($class->isInterface() ? 'interface' : 'class');
    $modifiers = [];
    if ($class->isFinal()) {
        $modifiers[] = 'final';
    }
    if ($class->isReadOnly()) {
        $modifiers[] = 'readonly';
    }
    $surface[] = sprintf(
        'symbol %s %s%s',
        $kind,
        $className,
        $modifiers === [] ? '' : ' '.implode(',', $modifiers),
    );

    foreach ($class->getReflectionConstants() as $constant) {
        if (
            !$constant->isPublic()
            || $constant->getDeclaringClass()->getName() !== $className
            || $constant->isEnumCase()
        ) {
            continue;
        }
        $surface[] = constantSignature($class, $constant);
    }

    if ($class->isEnum()) {
        $enum = new ReflectionEnum($className);
        foreach ($enum->getCases() as $case) {
            $value = constant($className.'::'.$case->getName());
            $surface[] = sprintf(
                'case %s::%s=%s',
                $className,
                $case->getName(),
                json_encode(
                    $value instanceof BackedEnum ? $value->value : $case->getName(),
                    JSON_THROW_ON_ERROR | JSON_UNESCAPED_SLASHES,
                ),
            );
        }
    }

    foreach ($class->getProperties(ReflectionProperty::IS_PUBLIC) as $property) {
        if ($property->getDeclaringClass()->getName() === $className) {
            $surface[] = propertySignature($class, $property);
        }
    }

    foreach ($class->getMethods(ReflectionMethod::IS_PUBLIC) as $method) {
        if ($method->getDeclaringClass()->getName() === $className) {
            $surface[] = methodSignature($class, $method);
        }
    }
}

sort($surface, SORT_STRING);
$actual = implode("\n", $surface)."\n";
$contractPath = dirname(__DIR__, 3).'/compat/php-api-v1.txt';
$expected = is_file($contractPath) ? file_get_contents($contractPath) : false;

if ($expected !== $actual) {
    fwrite(
        STDERR,
        "The PHP API v1 surface changed. Review it and update compat/php-api-v1.txt intentionally.\n\n".
        $actual,
    );
    exit(1);
}

fwrite(STDOUT, "PHP API v1 compatibility contract passed.\n");
