<?php

declare(strict_types=1);

use Pam\Desktop\App;
use Pam\Desktop\Component;
use Pam\Desktop\Database\SQLite;
use Pam\Desktop\Diagnostics\RuntimeError;
use Pam\Desktop\Diagnostics\RuntimeErrorPhase;
use Pam\Desktop\Diagnostics\StackFrame;
use Pam\Desktop\Internal\Runtime;
use Pam\Desktop\Internal\TreeEncoder;
use Pam\Desktop\ModuleResultStatus;
use Pam\Desktop\Modules\NativeModuleException;
use Pam\Desktop\Storage\Storage;
use Pam\Desktop\UI\Button;
use Pam\Desktop\UI\Text;

/**
 * Runtime error reporting contracts behind the native error overlay:
 * structured app-first frames, fatal phases, App::onError() hooks and
 * module failures routed to the caller's failure path.
 *
 * @var Closure(bool, string): void $assert
 */
$thisFile = RuntimeError::shortenPath(__FILE__);
$lastDiagnostic = static function (): array {
    $raw = (string) end(TestDiagnostics::$messages);
    if (!str_starts_with($raw, "PAMERR1\n")) {
        return [];
    }

    return json_decode(substr($raw, 8), true, 512, JSON_THROW_ON_ERROR);
};

// Path shortening strips device bundle prefixes and the project root.
$assert(
    RuntimeError::shortenPath('/data/user/0/dev.zechat/files/pam/releases/9f3a1c/app/Screens/Chat.php')
        === 'app/Screens/Chat.php'
    && RuntimeError::shortenPath('/data/data/dev.zechat/files/pam/ota-releases/abc123/index.php')
        === 'index.php'
    && RuntimeError::shortenPath(getcwd().'/app/Chat.php') === 'app/Chat.php'
    && RuntimeError::shortenPath('/elsewhere/file.php') === '/elsewhere/file.php',
    'Runtime error paths must be app-relative.',
);

// Event callback errors are non-fatal, structured and app-frame-first.
TestDiagnostics::$messages = [];
$reported = [];
$failingEvent = new class extends Component {
    public function render(): \Pam\Desktop\Element
    {
        return Button::make('Fail')->onPress(static function (): void {
            throw new RuntimeException('Event handler exploded.');
        });
    }
};
App::run($failingEvent);
App::onError(static function (Throwable $error, RuntimeError $report) use (&$reported): void {
    $reported[] = [$error->getMessage(), $report];
});
App::onError(static function (): void {
    throw new LogicException('A broken crash reporter must be ignored.');
});
$failingFrame = (new TreeEncoder())->encode($failingEvent->render());
[$eventNode, $eventKind] = array_map('intval', explode(':', (string) array_key_first($failingFrame['callbacks'])));
Runtime::dispatchEvent($eventNode, $eventKind, '');
$diagnostic = $lastDiagnostic();
$assert(
    ($diagnostic['version'] ?? null) === 2
        && $diagnostic['type'] === RuntimeException::class
        && $diagnostic['message'] === 'Event handler exploded.'
        && $diagnostic['phase'] === 'event'
        && $diagnostic['fatal'] === false
        && $diagnostic['file'] === $thisFile
        && str_ends_with($thisFile, 'tests/error_reporting.php')
        && !str_starts_with($thisFile, '/')
        && !str_contains($diagnostic['trace'], "\0")
        && str_contains($diagnostic['frames'][0]['call'], '@anonymous::{closure}()')
        && is_int($diagnostic['appFrame'])
        && $diagnostic['frames'][$diagnostic['appFrame']]['kind'] === StackFrame::APP
        && $diagnostic['frames'][$diagnostic['appFrame']]['file'] === $thisFile
        && in_array(StackFrame::FRAMEWORK, array_column($diagnostic['frames'], 'kind'), true)
        && is_string($diagnostic['fingerprint']) && $diagnostic['fingerprint'] !== ''
        && !str_contains($diagnostic['trace'], getcwd().'/'),
    'Event errors must emit non-fatal structured diagnostics with app frames first and short paths.',
);
$assert(
    is_array($diagnostic['snippet'] ?? null)
        && $diagnostic['snippet']['file'] === $thisFile
        && count(array_filter(
            $diagnostic['snippet']['lines'],
            static fn (string $line): bool => str_contains($line, 'Event handler exploded.'),
        )) === 1,
    'Development diagnostics must include the app source snippet around the failing line.',
);
$assert(
    count($reported) === 1
        && $reported[0][0] === 'Event handler exploded.'
        && $reported[0][1]->phase === RuntimeErrorPhase::Event
        && !$reported[0][1]->fatal(),
    'App::onError() must receive every uncaught runtime error once, even with a broken listener.',
);
$assert(Runtime::lastFrame() !== null, 'A non-fatal event error must keep the committed UI.');
Runtime::shutdown();

// Render errors are fatal whatever path triggered the render.
TestDiagnostics::$messages = [];
$explode = false;
$fatalRoot = new class ($explode) extends Component {
    public function __construct(private bool &$explode)
    {
    }

    public function render(): \Pam\Desktop\Element
    {
        if ($this->explode) {
            throw new DomainException('Render exploded.');
        }

        return Button::make('Break')->onPress(function (): void {
            $this->explode = true;
        });
    }
};
App::run($fatalRoot);
$fatalFrame = (new TreeEncoder())->encode($fatalRoot->render());
[$fatalNode, $fatalKind] = array_map('intval', explode(':', (string) array_key_first($fatalFrame['callbacks'])));
Runtime::dispatchEvent($fatalNode, $fatalKind, '');
$diagnostic = $lastDiagnostic();
$assert(
    ($diagnostic['message'] ?? null) === 'Render exploded.'
        && $diagnostic['phase'] === 'render'
        && $diagnostic['fatal'] === true,
    'Errors thrown while rendering must be reported as fatal render errors.',
);
Runtime::shutdown();

// Boot errors are fatal.
TestDiagnostics::$messages = [];
App::run(static function (): never {
    throw new LogicException('Boot exploded.');
});
$diagnostic = $lastDiagnostic();
$assert(
    ($diagnostic['message'] ?? null) === 'Boot exploded.'
        && $diagnostic['phase'] === 'boot'
        && $diagnostic['fatal'] === true,
    'Errors thrown by the first render must be reported as fatal boot errors.',
);
Runtime::shutdown();

// Module failures go to the caller's failure path instead of the overlay.
App::run(static fn (): Text => Text::make('Module results'));
TestDiagnostics::$messages = [];
TestDiagnostics::$moduleCall = null;
$routed = null;
$rows = null;
SQLite::query(
    'chat.db',
    'SELECT body FROM messages',
    [],
    static function (array $result) use (&$rows): void {
        $rows = $result;
    },
    static function (NativeModuleException $error) use (&$routed): void {
        $routed = $error;
    },
);
$request = TestDiagnostics::$moduleCall['requestId'] ?? 0;
Runtime::dispatchModuleResult($request, ModuleResultStatus::Failure->value, 'Native module value is too large');
$assert(
    $routed instanceof NativeModuleException
        && $routed->module === 'sqlite'
        && $routed->method === 'query'
        && $routed->getMessage() === 'Native module value is too large'
        && $rows === null
        && TestDiagnostics::$messages === [],
    'SQLite failures must reach the onError callback and never the runtime error overlay.',
);

// Without a failure path, the failure is a typed, non-fatal module error.
TestDiagnostics::$moduleCall = null;
SQLite::execute('chat.db', 'DELETE FROM messages');
Runtime::dispatchModuleResult(
    TestDiagnostics::$moduleCall['requestId'] ?? 0,
    ModuleResultStatus::Failure->value,
    'database is locked',
);
$diagnostic = $lastDiagnostic();
$assert(
    ($diagnostic['type'] ?? null) === NativeModuleException::class
        && $diagnostic['message'] === 'database is locked'
        && $diagnostic['phase'] === 'module'
        && $diagnostic['fatal'] === false,
    'Unhandled module failures must report a non-fatal NativeModuleException.',
);

TestDiagnostics::$messages = [];
TestDiagnostics::$typedCall = null;
$storageError = null;
Storage::get(
    'profile',
    static function (?string $_value): void {},
    static function (NativeModuleException $error) use (&$storageError): void {
        $storageError = $error;
    },
);
Runtime::dispatchModuleResult(
    TestDiagnostics::$typedCall['requestId'] ?? 0,
    ModuleResultStatus::Failure->value,
    'Storage unavailable',
);
$assert(
    $storageError instanceof NativeModuleException
        && $storageError->getMessage() === 'Storage unavailable'
        && TestDiagnostics::$messages === [],
    'Storage failures must reach the onError callback.',
);
Runtime::shutdown();
TestDiagnostics::$messages = [];
TestDiagnostics::$moduleCall = null;
TestDiagnostics::$typedCall = null;

// An error raised inside a template keeps its own type, message and origin;
// the template is one frame of its stack. A template error of its own points
// at the template, with a message free of the location.
$inner = new DomainException('Total is not ready.');
$report = RuntimeError::from(
    new \Pam\Desktop\TemplateException($inner->getMessage(), __FILE__, 7, 3, $inner),
    RuntimeErrorPhase::Render,
);
$templateFrames = array_values(array_filter($report->frames, static fn (StackFrame $frame): bool => $frame->call === 'template'));
$assert(
    $report->type === DomainException::class
        && $report->message === 'Total is not ready.'
        && $report->file === $thisFile
        && $report->line === $inner->getLine()
        && count($templateFrames) === 1
        && $templateFrames[0]->line === 7,
    'A template must report the error raised inside it, with the template as a frame.',
);
$report = RuntimeError::from(new \Pam\Desktop\TemplateException('Unknown variable $total', __FILE__, 7, 3), RuntimeErrorPhase::Render);
$assert(
    $report->type === \Pam\Desktop\TemplateException::class
        && $report->message === 'Unknown variable $total'
        && $report->line === 7
        && $report->column === 3
        && $report->frames[$report->appFrame]->call === 'template'
        && ($report->snippet['line'] ?? null) === 7,
    'A template error must point at the template line and column.',
);

// A PHP fatal error has no stack: the report is where it stopped.
$report = RuntimeError::from(new ErrorException('Allowed memory size exhausted', 0, E_ERROR, __FILE__, 3), RuntimeErrorPhase::Worker);
$assert(
    $report->fatal()
        && count($report->frames) === 1
        && $report->frames[0]->call === 'fatal error'
        && $report->file === $thisFile
        && $report->line === 3,
    'A fatal error must be reported at the line where PHP stopped.',
);
