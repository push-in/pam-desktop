<?php

declare(strict_types=1);

use Pam\Desktop\App;
use Pam\Desktop\Appearance;
use Pam\Desktop\AppearanceMode;
use Pam\Desktop\AppState;
use Pam\Desktop\Composables\SharedSource;
use Pam\Desktop\Composables\Sources;
use Pam\Desktop\DeviceClass;
use Pam\Desktop\EventKind;
use Pam\Desktop\Internal\Runtime;
use Pam\Desktop\Internal\Wire;
use Pam\Desktop\ModuleResultStatus;
use Pam\Desktop\NetworkType;
use Pam\Desktop\System\Keyboard;
use Pam\Desktop\TextScalePolicy;
use Pam\Desktop\UserInterfaceAppearance;
use Pam\Desktop\WindowMetrics;

use function Pam\Desktop\computed;
use function Pam\Desktop\effectScope;
use function Pam\Desktop\useAppState;
use function Pam\Desktop\useColorScheme;
use function Pam\Desktop\useFontScale;
use function Pam\Desktop\useKeyboard;
use function Pam\Desktop\useNetwork;
use function Pam\Desktop\useSafeAreaInsets;
use function Pam\Desktop\useWindowDimensions;
use function Pam\Desktop\watch;

$assert = require __DIR__.'/arch_v2_bootstrap.php';

/** @return array{0: int, 1: string, 2: string}|null last native module call: request id, module, method */
$lastCall = static function (): ?array {
    if (class_exists('TestDiagnostics', false) && TestDiagnostics::$moduleCall !== null) {
        $call = TestDiagnostics::$moduleCall;

        return [$call['requestId'], $call['module'], $call['method']];
    }
    $calls = $GLOBALS['pamArchV2NativeCalls'] ?? [];
    $call = end($calls);

    return $call === false ? null : [$call[0], $call[1], $call[2]];
};
$count = static fn (string $class, string $property): int => count((new ReflectionProperty($class, $property))->getValue());

Sources::reset();
Appearance::resetRuntime();
Keyboard::resetRuntime();
TextScalePolicy::reset();
$appearanceBaseline = $count(Appearance::class, 'listeners');
$windowBaseline = Runtime::windowMetricsListeners();

$values = [];
$scope = effectScope(static function () use (&$values, $assert): void {
    $values['scheme'] = useColorScheme();
    $values['insets'] = useSafeAreaInsets();
    $values['window'] = useWindowDimensions();
    $values['font'] = useFontScale();
    $values['keyboard'] = useKeyboard();
    $values['app'] = useAppState();
    $values['network'] = useNetwork();
    // Several consumers of one kind share one platform subscription.
    useSafeAreaInsets();
    useWindowDimensions();
});
$watchCall = $lastCall();
$assert(
    SharedSource::active() === ['appearance' => 1, 'window' => 5, 'keyboard' => 1, 'appState' => 1, 'network' => 1]
        && Runtime::windowMetricsListeners() === $windowBaseline + 1
        && $count(Appearance::class, 'listeners') === $appearanceBaseline + 1,
    'Composables must share one subscription per kind: '.json_encode(SharedSource::active()),
);

// useColorScheme(): value/mode follow Appearance; set() accepts 'dark'|'light'|'system'.
$scheme = $values['scheme'];
$schemes = [];
$watcher = watch(static fn (): string => $scheme->value.'/'.$scheme->mode, static function (string $next) use (&$schemes): void {
    $schemes[] = $next;
});
$scheme->set('dark');
$scheme->set('system');
$scheme->set(AppearanceMode::Light);
$watcher->dispose();
$assert($schemes === ['dark/dark', 'light/system', 'light/light'] && $scheme->dark === false && $scheme->preference === AppearanceMode::Light, 'useColorScheme() must be reactive and settable: '.json_encode($schemes));
$badMode = false;
try {
    $scheme->set('sepia');
} catch (InvalidArgumentException) {
    $badMode = true;
}
$assert($badMode, 'useColorScheme()->set() must reject unknown modes.');

// Window metrics: insets, dimensions and font scale follow host updates, without polling.
$insets = $values['insets'];
$bottom = computed(static fn (): float => $insets->bottom);
Runtime::dispatchEvent(0, EventKind::Dimensions->value, Wire::map([
    'width' => 820.0, 'height' => 600.0, 'density' => 2.0, 'fontScale' => 1.8,
    'safeAreaTop' => 24.0, 'safeAreaBottom' => 34.0, 'safeAreaLeft' => 0.0, 'safeAreaRight' => 0.0,
]));
$window = $values['window'];
$font = $values['font'];
TextScalePolicy::set(maximum: 1.5, display: 1.2, displayFontSize: 20);
$assert(
    $insets->top === 24.0 && $bottom->value === 34.0 && $insets->toArray() === ['top' => 24.0, 'right' => 0.0, 'bottom' => 34.0, 'left' => 0.0],
    'useSafeAreaInsets() must follow the host window.',
);
$assert(
    $window->width === 820.0 && $window->landscape && $window->scale === 2.0 && $window->deviceClass === DeviceClass::Medium,
    'useWindowDimensions() must follow the host window.',
);
$assert($font->value === 1.8 && $font->effective === 1.5 && $font->display === 1.2 && $font->for(12.0) === 1.5, 'useFontScale() must apply TextScalePolicy caps.');
TextScalePolicy::reset();
$assert(
    $window->heightClass === DeviceClass::Medium && $font->clamp(1.0, 1.3) === 1.3 && $font->scaled(10.0, 1.5) === 15.0 && $font->clamp(2.0) === 2.0,
    'heightClass and useFontScale()->clamp()/scaled() must follow the Material classes and Flutter TextScaler.clamp.',
);

// Each window property is its own signal (Flutter MediaQuery.sizeOf): a
// reader of the width is not re-run when only the height or the insets change.
$widthRuns = 0;
$heightRuns = 0;
$widthReader = computed(static function () use ($window, &$widthRuns): float {
    $widthRuns++;

    return $window->width;
});
$heightReader = computed(static function () use ($window, &$heightRuns): float {
    $heightRuns++;

    return $window->height;
});
$widthReader->value;
$heightReader->value;
Runtime::dispatchEvent(0, EventKind::Dimensions->value, Wire::map([
    'width' => 820.0, 'height' => 340.0, 'density' => 2.0, 'fontScale' => 1.8,
    'safeAreaTop' => 24.0, 'safeAreaBottom' => 0.0, 'safeAreaLeft' => 0.0, 'safeAreaRight' => 0.0,
]));
$widthReader->value;
$heightReader->value;
$assert(
    $widthRuns === 1 && $heightRuns === 2 && $window->heightClass === DeviceClass::Compact && $insets->bottom === 0.0,
    'Window readers must re-run only for the property they read: '.json_encode([$widthRuns, $heightRuns]),
);

// useKeyboard(): settled show/hide/resize.
$keyboard = $values['keyboard'];
Runtime::dispatchEvent(0, EventKind::KeyboardVisibility->value, '1;291.5;250');
$assert($keyboard->visible && $keyboard->height === 291.5 && $keyboard->durationMs === 250, 'useKeyboard() must report the settled keyboard.');
Runtime::dispatchEvent(0, EventKind::KeyboardVisibility->value, '0;0;200');
$assert(!$keyboard->visible && $keyboard->height === 0.0, 'useKeyboard() must report the keyboard hiding.');

// useAppState(): foreground/background transitions.
$app = $values['app'];
Runtime::dispatchEvent(0, EventKind::AppState->value, (string) AppState::Background->value);
$assert($app->background && !$app->active && $app->value === AppState::Background, 'useAppState() must follow backgrounding.');
Runtime::dispatchEvent(0, EventKind::AppState->value, (string) AppState::Active->value);
$assert($app->active, 'useAppState() must follow the return to the foreground.');

// useNetwork(): assumed online until the host pushes a status; pushes go through the device watch.
$network = $values['network'];
$assert($network->online && !$network->known && $network->type === null, 'useNetwork() must assume online until the first report.');
$assert($watchCall !== null && $watchCall[1] === 'device' && $watchCall[2] === 'watch', 'useNetwork() must start one device watch.');
Runtime::dispatchModuleResult($watchCall[0], ModuleResultStatus::Success->value, Wire::map(['subscription' => 9]));
$nextCall = $lastCall();
Runtime::dispatchModuleResult($nextCall[0], ModuleResultStatus::Success->value, Wire::map(['networkType' => NetworkType::None->value, 'expensiveNetwork' => false]));
$assert(!$network->online && !$network->value && $network->known && $network->type === NetworkType::None, 'useNetwork() must follow host status pushes.');

// Disposing the scope releases every subscription, including the native watch.
$scope->dispose();
$stopCall = $lastCall();
$assert(
    SharedSource::active() === []
        && Runtime::windowMetricsListeners() === $windowBaseline
        && $count(Appearance::class, 'listeners') === $appearanceBaseline
        && $stopCall !== null && $stopCall[1] === 'device' && $stopCall[2] === 'stop',
    'Disposing must release every composable subscription and stop the native device watch: '.json_encode(SharedSource::active()),
);

// Outside a scope a composable lives for the application (index.php globals).
$global = useColorScheme();
$assert(SharedSource::active() === ['appearance' => 1] && $global->value === 'light', 'A composable outside any scope must live as long as the app.');
$readOnly = false;
try {
    $global->value = 'dark';
} catch (LogicException) {
    $readOnly = true;
}
$assert($readOnly, 'Composable properties must be read-only.');
// Runtime::shutdown() (hot reload, tests) stops every shared source.
useKeyboard();
$assert(SharedSource::active() === ['appearance' => 1, 'keyboard' => 1], 'App-lifetime composables keep their sources until shutdown.');
Runtime::shutdown();
$assert(
    SharedSource::active() === [] && $count(Appearance::class, 'listeners') === 0
        && Runtime::windowMetricsListeners() === 0,
    'Runtime::shutdown() must reset SharedSource and release its platform listeners: '.json_encode(SharedSource::active()),
);
$fresh = useColorScheme();
$assert(SharedSource::active() === ['appearance' => 1] && $fresh->value === 'light', 'A composable after shutdown starts a fresh source.');
Sources::reset();
Appearance::resetRuntime();
Keyboard::resetRuntime();

echo "ARCH_V2_COMPOSABLES_OK scheme insets window font keyboard app-state network shared no-leak\n";
