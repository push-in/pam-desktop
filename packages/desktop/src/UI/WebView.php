<?php

declare(strict_types=1);

namespace Pam\Desktop\UI;

use Closure;
use InvalidArgumentException;
use Pam\Desktop\Element;
use Pam\Desktop\EventKind;
use Pam\Desktop\Internal\Wire;
use Pam\Desktop\NodeKind;
use Pam\Desktop\PropKey;

use function strlen;

/**
 * An embedded web page, rendered by the web engine of the operating system
 * (WebKitGTK on Linux, WKWebView on macOS, WebView2 on Windows).
 *
 * ```php
 * WebView::make('https://example.com/help')
 *     ->allowedHosts('example.com')
 *     ->onMessage(fn (string $message, string $url) => $this->handle($message));
 * ```
 *
 * Pages talk to the application with `window.PamNative.postMessage(text)`;
 * the application talks to the page by changing `injectedJavaScript()`.
 */
final class WebView extends Element
{
    private const MAX_BYTES = 1_048_576;

    /** @param string $source a `https`, `http` or `file` URL, or inline HTML starting with `<` */
    public static function make(string $source): self
    {
        return (new self(NodeKind::WebView))
            ->withProperty(PropKey::WebViewSource, self::validSource($source))
            ->withProperty(PropKey::WebViewJavaScriptEnabled, true)
            ->withProperty(PropKey::WebViewDomStorageEnabled, true)
            ->withProperty(PropKey::WebViewAllowsInlineMedia, true);
    }

    public function source(string $source): self
    {
        return $this->withProperty(PropKey::WebViewSource, self::validSource($source));
    }

    public function javaScriptEnabled(bool $enabled = true): self
    {
        return $this->withProperty(PropKey::WebViewJavaScriptEnabled, $enabled);
    }

    /** Local and session storage. */
    public function domStorageEnabled(bool $enabled = true): self
    {
        return $this->withProperty(PropKey::WebViewDomStorageEnabled, $enabled);
    }

    /** `null` keeps the engine's own user agent. */
    public function userAgent(?string $userAgent): self
    {
        return $this->withProperty(PropKey::WebViewUserAgent, $userAgent ?? '');
    }

    /**
     * A script evaluated after every page load, and at once when it changes
     * on a loaded page. To run the same script again, make the text differ.
     */
    public function injectedJavaScript(string $script): self
    {
        if (strlen($script) > self::MAX_BYTES) {
            throw new InvalidArgumentException('Injected JavaScript cannot exceed one mebibyte.');
        }

        return $this->withProperty(PropKey::WebViewInjectedJavaScript, $script);
    }

    /** Media may start without a click or key press. */
    public function allowsInlineMedia(bool $enabled = true): self
    {
        return $this->withProperty(PropKey::WebViewAllowsInlineMedia, $enabled);
    }

    /**
     * Limits main-frame navigation to these hosts (exact match, no
     * wildcards). With no hosts, every page is allowed.
     */
    public function allowedHosts(string ...$hosts): self
    {
        $normalized = [];
        foreach ($hosts as $host) {
            $host = strtolower(trim($host));
            if (preg_match('/^[a-z0-9.-]{1,253}$/D', $host) !== 1) {
                throw new InvalidArgumentException("WebView host {$host} is invalid.");
            }
            $normalized[$host] = true;
        }

        return $this->withProperty(PropKey::WebViewAllowedHosts, implode("\n", array_keys($normalized)));
    }

    public function scrollEnabled(bool $enabled = true): self
    {
        return $this->withProperty(PropKey::ScrollEnabled, $enabled);
    }

    /** A page finished loading. */
    public function onLoad(Closure $handler): self
    {
        return $this->withEvent(EventKind::WebViewLoad, static fn (mixed $payload = ''): mixed => $handler());
    }

    /** @param Closure(string): mixed $handler receives the failure message */
    public function onError(Closure $handler): self
    {
        return $this->withEvent(
            EventKind::WebViewError,
            static fn (string $payload): mixed => $handler((string) (Wire::decodeMap($payload)['message'] ?? '')),
        );
    }

    /** @param Closure(string, string): mixed $handler receives the message and the page URL */
    public function onMessage(Closure $handler): self
    {
        return $this->withEvent(
            EventKind::WebViewMessage,
            static function (string $payload) use ($handler): mixed {
                $values = Wire::decodeMap($payload);

                return $handler((string) ($values['message'] ?? ''), (string) ($values['url'] ?? ''));
            },
        );
    }

    private static function validSource(string $source): string
    {
        $length = strlen($source);
        if ($length < 1 || $length > self::MAX_BYTES) {
            throw new InvalidArgumentException('WebView source must hold between 1 byte and one mebibyte.');
        }
        if (str_starts_with(ltrim($source), '<')) {
            return $source;
        }
        $scheme = strtolower((string) parse_url($source, PHP_URL_SCHEME));
        if (!in_array($scheme, ['https', 'http', 'file'], true)) {
            throw new InvalidArgumentException('WebView source must be an https, http or file URL, or inline HTML.');
        }

        return $source;
    }
}
