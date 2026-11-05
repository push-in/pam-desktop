<?php

declare(strict_types=1);

use Pam\Desktop\App;
use Pam\Desktop\Attributes\State;
use Pam\Desktop\Component;
use Pam\Desktop\Style;
use Pam\Desktop\UI\Button;
use Pam\Desktop\UI\Column;
use Pam\Desktop\UI\Row;
use Pam\Desktop\UI\Text;
use Pam\Desktop\UI\WebView;

require __DIR__.'/../../vendor/autoload.php';

/**
 * Exercises the web view: an inline page that talks to PHP through
 * `PamNative.postMessage`, a script injected from state, a remote page and
 * the allowed-host policy.
 */
final class Web extends Component
{
    private const PAGE = <<<'HTML'
        <!doctype html>
        <meta charset="utf-8">
        <style>
          body { font: 16px system-ui, sans-serif; margin: 0; padding: 24px; background: #12161c; color: #e8ecf3; }
          button { font: inherit; padding: 10px 16px; border: 0; border-radius: 10px; background: #2f6bff; color: #fff; cursor: pointer; }
          #count { font-size: 44px; font-weight: 700; margin: 12px 0; }
        </style>
        <h2>Inline page</h2>
        <div id="count">0</div>
        <button onclick="PamNative.postMessage('clicked ' + (++window.clicks))">Send a message to PHP</button>
        <script>
          window.clicks = 0;
          window.setCount = function (value) { document.getElementById('count').textContent = value; };
          PamNative.postMessage('page ready');
        </script>
        HTML;

    #[State]
    public string $status = 'Loading…';

    #[State]
    public int $count = 0;

    #[State]
    public bool $remote = false;

    public function render(): Column
    {
        $view = $this->remote
            ? WebView::make('https://example.com/')->allowedHosts('example.com')
            : WebView::make(self::PAGE)->injectedJavaScript("window.setCount({$this->count});");

        return Column::make(
            Text::make('WebView')->style(new Style(fontSize: 24, fontWeight: 700, textColor: 0xFF12161C)),
            $view
                ->key($this->remote ? 'remote' : 'inline')
                ->onLoad(function (): void {
                    $this->status = 'Page loaded';
                })
                ->onError(function (string $message): void {
                    $this->status = 'Error: '.$message;
                })
                ->onMessage(function (string $message, string $url): void {
                    $this->status = "Message: {$message}";
                })
                ->style(new Style(flexGrow: 1, borderRadius: 16, backgroundColor: 0xFF12161C)),
            Row::make(
                Text::make($this->status)->style(new Style(fontSize: 13, textColor: 0xFF5B6472, flexGrow: 1)),
                Button::make('Count +1 (PHP to page)')->onPress(function (): void { $this->count++; }),
                Button::make($this->remote ? 'Inline page' : 'example.com')->onPress(function (): void {
                    $this->remote = !$this->remote;
                    $this->status = 'Loading…';
                }),
            )->style(new Style(gap: 8, alignItems: Pam\Desktop\Align::Center)),
        )->style(new Style(flexGrow: 1, padding: 28, gap: 14, backgroundColor: 0xFFF7F8FA));
    }
}

App::run(new Web());
