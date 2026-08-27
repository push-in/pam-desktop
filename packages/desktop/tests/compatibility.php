<?php

declare(strict_types=1);

require dirname(__DIR__).'/vendor/autoload.php';

/**
 * Keep this representation deliberately language-neutral and line-oriented.
 * Reviewing an intentional additive API change should produce a small snapshot
 * diff, while removals and signature changes fail the compatibility suite.
 */
function parameterSignature(ReflectionClass $class, ReflectionParameter $parameter): string
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
        $type = (string) $parameter->getType();
        if ($type === $class->getName()) {
            $type = 'self';
        } elseif ($type === '?'.$class->getName()) {
            $type = '?self';
        }
        $signature .= ':'.$type;
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
        static fn (ReflectionParameter $parameter): string => parameterSignature($class, $parameter),
        $method->getParameters(),
    );
    $returnType = $method->hasReturnType() ? (string) $method->getReturnType() : '-';
    // PHP 8.5 resolves a declared `self` return to the declaring class while
    // older supported engines preserve the source spelling. Keep the public
    // API contract stable across engines by using the semantic spelling.
    if ($returnType === $class->getName()) {
        $returnType = 'self';
    } elseif ($returnType === '?'.$class->getName()) {
        $returnType = '?self';
    }

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

$sourceRoot = dirname(__DIR__).'/src';
$sourceFiles = [];
$iterator = new RecursiveIteratorIterator(new RecursiveDirectoryIterator($sourceRoot));
foreach ($iterator as $sourceFile) {
    if ($sourceFile->isFile() && $sourceFile->getExtension() === 'php') {
        $sourceFiles[] = $sourceFile->getPathname();
    }
}
sort($sourceFiles, SORT_STRING);

$surface = [];
foreach ($sourceFiles as $sourceFile) {
    $relative = substr($sourceFile, strlen($sourceRoot) + 1, -4);
    $className = 'Pam\\Desktop\\'.str_replace(DIRECTORY_SEPARATOR, '\\', $relative);
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
    if (getenv('PAM_UPDATE_COMPATIBILITY') === '1') {
        if (file_put_contents($contractPath, $actual) === false) {
            throw new RuntimeException('Could not update the PHP API compatibility snapshot.');
        }
        fwrite(STDOUT, "PHP API v1 compatibility contract updated.\n");
        exit(0);
    }
    fwrite(
        STDERR,
        "The PHP API v1 surface changed. Review it and update compat/php-api-v1.txt intentionally.\n\n".
        $actual,
    );
    exit(1);
}

fwrite(STDOUT, "PHP API v1 compatibility contract passed.\n");
