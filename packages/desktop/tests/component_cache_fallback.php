<?php

declare(strict_types=1);

use Pam\Desktop\Internal\PamPhpCompiler;

// An application installed read-only (a system package, an AppImage) cannot
// write the component cache it names next to itself: components compile
// into a private cache below PAM_DESKTOP_CACHE_DIR instead of failing boot.
$fallbackRoot = sys_get_temp_dir().'/pam-desktop-cache-fallback-'.getmypid();
exec('rm -rf '.escapeshellarg($fallbackRoot));
mkdir($fallbackRoot.'/app/src', 0o755, true);
mkdir($fallbackRoot.'/user-cache', 0o755, true);
file_put_contents($fallbackRoot.'/app/src/Tag.pam.php', <<<'PAM'
<?php

declare(strict_types=1);

namespace Pam\Desktop\Tests\CacheFallback;

use Pam\Desktop\Component;

final class Tag extends Component
{
    public function __construct(public string $label = 'Tag')
    {
    }
}
?>

<template>
    <Text>{{ $label }}</Text>
</template>
PAM);
chmod($fallbackRoot.'/app', 0o555);
$previousCache = getenv('PAM_DESKTOP_CACHE_DIR');
putenv('PAM_DESKTOP_CACHE_DIR='.$fallbackRoot.'/user-cache');
try {
    if (is_writable($fallbackRoot.'/app')) {
        echo "component cache fallback: skipped (running as a user who can write anywhere)\n";
    } else {
        $components = PamPhpCompiler::compileDirectory($fallbackRoot.'/app/src', $fallbackRoot.'/app/.pam-desktop/components');
        if (count($components) !== 1 || !str_starts_with($components[0]->classFile, $fallbackRoot.'/user-cache/components/')) {
            throw new RuntimeException('A read-only application must compile its components into the user cache.');
        }
        if (is_dir($fallbackRoot.'/app/.pam-desktop')) {
            throw new RuntimeException('Nothing may be written inside a read-only application.');
        }
        // A later compile of one file follows the same redirect.
        $again = PamPhpCompiler::compileFile($fallbackRoot.'/app/src/Tag.pam.php', $fallbackRoot.'/app/.pam-desktop/components');
        if ($again->classFile !== $components[0]->classFile) {
            throw new RuntimeException('Recompiling a component must reuse the redirected cache.');
        }
        echo "component cache fallback: passed\n";
    }
} finally {
    putenv($previousCache === false ? 'PAM_DESKTOP_CACHE_DIR' : 'PAM_DESKTOP_CACHE_DIR='.$previousCache);
    chmod($fallbackRoot.'/app', 0o755);
    exec('rm -rf '.escapeshellarg($fallbackRoot));
}
