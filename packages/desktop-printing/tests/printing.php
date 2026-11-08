<?php

declare(strict_types=1);
use Pam\DesktopPrinting\Document;
use Pam\DesktopPrinting\TextBlock;
use Pam\DesktopPrinting\Printing;
use Pam\DesktopPrinting\PrintResult;
use Pam\Desktop\Internal\Wire;
use Pam\Desktop\ModuleResultStatus;
use Pam\Desktop\Modules\NativeModules;
use Pam\Desktop\Modules\NativeModuleTransport;

$loader = require dirname(__DIR__, 3).'/vendor/autoload.php';
$loader->addPsr4('Pam\\DesktopPrinting\\', dirname(__DIR__).'/src');
function printCheck(bool $value, string $message): void { if (!$value) throw new RuntimeException($message); }
$document = new Document('Comprovante', 226.77, 280, [[new TextBlock('Café — R$ 19,90', 12, 12, 200, 30)]]);
$data = json_decode($document->json(), true, flags: JSON_THROW_ON_ERROR);
printCheck($data['pages'][0][0]['align'] === 1 && $data['pages'][0][0]['text'] === 'Café — R$ 19,90', 'Unicode and integer alignment survive wire JSON.');
foreach ([static fn () => new Document('', 200, 200, [[]]), static fn () => new Document('x', 200, 200, []), static fn () => new Document('x', 200, 200, [[new TextBlock('overflow', 150, 12, 100, 30)]]), static fn () => new TextBlock('x', INF, 0, 10, 10)] as $invalid) {
    try { $invalid(); throw new LogicException('Accepted invalid print document.'); } catch (InvalidArgumentException $expected) {}
}
$calls = []; $callback = null;
NativeModules::useTransport(new class($calls, $callback) implements NativeModuleTransport {
    public function __construct(private array &$calls, private ?Closure &$callback) {}
    public function invoke(int $requestId, string $module, string $method, string $payload, Closure $complete): void { $this->calls[] = [$module, $method, Wire::decodeMap($payload)]; $this->callback = $complete; }
});
$result = null; $error = null;
$success = static function (PrintResult $value) use (&$result): void { $result = $value; };
$failure = static function (Throwable $value) use (&$error): void { $error = $value; };
Printing::print($document, $success, $failure);
printCheck($calls[0][0] === 'printing' && $calls[0][1] === 'print' && $calls[0][2]['document'] === $document->json(), 'Library invokes native print module without HTML.');
Printing::print($document, $success, $failure);
printCheck(count($calls) === 1 && $error !== null, 'Concurrent dialog rejected.');
$callback(ModuleResultStatus::Success, Wire::map(['result' => PrintResult::Cancelled->value]));
printCheck($result === PrintResult::Cancelled, 'User cancel is distinct from successful acceptance.');
Printing::print($document, $success, $failure);
$callback(ModuleResultStatus::Failure, 'Printer unavailable');
printCheck($error->getMessage() === 'Printer unavailable', 'Native failures are visible.');
Printing::print($document, $success, $failure);
$callback(ModuleResultStatus::Success, Wire::map(['result' => PrintResult::Accepted->value]));
printCheck($result === PrintResult::Accepted, 'Completion unlocks next job.');
NativeModules::useTransport(null);
echo "Printing document bounds, Unicode, native transport and dialog lifecycle passed.\n";
$receipt = (new Pam\DesktopPrinting\Receipt('Unicode receipt'))->text(str_repeat('Café 漢字 ', 30))->pair('2x Café especial', 'R$ 99,90')->separator()->document();
$receiptText = implode('', array_map(static fn (TextBlock $block): string => $block->text, $receipt->pages[0]));
printCheck(str_contains($receiptText, str_repeat('Café 漢字 ', 30)), 'Receipt wrapping preserves all Unicode content without truncation.');
printCheck(count($receipt->pages[0]) > 5 && $receipt->height <= 8500, 'Receipt lines grow the paper within bounds.');
$large = new Pam\DesktopPrinting\Receipt('Multiple pages');
$large->text(str_repeat("Item\n", 1000));
printCheck(count($large->document()->pages) === 2, 'Large receipts paginate instead of exceeding paper height.');
echo "Receipt Unicode wrapping and bounded pagination passed.\n";

printCheck(Document::fromJson($document->json())->json() === $document->json(), "Stored native document restores Unicode and geometry.");
foreach (['{}', str_repeat("x", 2_097_153)] as $invalidJson) {
    try { Document::fromJson($invalidJson); throw new LogicException("Accepted invalid stored document."); } catch (InvalidArgumentException|JsonException $expected) {}
}
echo "Stored document validation passed.\n";
$fixed=(new Pam\DesktopPrinting\Receipt('A4 report',595,36))->text(str_repeat("José; café\n",120))->document(842);
printCheck(count($fixed->pages)>1&&$fixed->height===842.0,'Fixed paper height paginates A4');
$lines=0;foreach($fixed->pages as $page){foreach($page as $block){printCheck($block->y+$block->height<=806,'A4 bottom margin preserved');if($block->text==='José; café')++$lines;}}
printCheck($lines===120,'A4 preserves every Unicode line across page breaks');
echo "Fixed paper pagination preserves Unicode and margins.\n";
