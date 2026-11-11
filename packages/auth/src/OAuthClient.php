<?php

declare(strict_types=1);

namespace Pam\Desktop\Auth;

use Closure;
use InvalidArgumentException;
use Pam\Desktop\Http\Http;
use Pam\Desktop\Http\HttpResponse;
use Pam\Desktop\Storage\SecureStorage;
use Pam\Desktop\System\Linking;

/**
 * Sign-in with OAuth 2.0 and OpenID Connect the way native apps should do it
 * (RFC 8252): the system browser opens the provider's page, the provider
 * sends the browser back to the app's own URL scheme, and the app exchanges
 * the code with PKCE. Tokens live in the system keychain and are refreshed
 * before they expire.
 *
 * ```php
 * $this->auth = new OAuthClient(
 *     authorizeUrl: 'https://accounts.example.com/oauth/authorize',
 *     tokenUrl: 'https://accounts.example.com/oauth/token',
 *     clientId: 'desktop-app',
 *     redirectUri: 'myapp://oauth/callback',      // a scheme listed in pam-desktop.json urlSchemes
 *     scopes: ['openid', 'profile', 'email', 'offline_access'],
 * );
 * $this->auth->signIn(fn (Tokens $tokens) => $this->user = $tokens->claims()['name'] ?? null);
 * $this->auth->accessToken(fn (?string $token) => Http::get($api, $show, ['Authorization' => "Bearer {$token}"]));
 * ```
 */
final class OAuthClient
{
    private ?int $claim = null;
    private ?Tokens $cached = null;

    /**
     * @param list<string> $scopes
     * @param array<string, string> $parameters extra authorize parameters (audience, prompt, login_hint…)
     * @param (Closure(string): void)|null $openBrowser opens the authorize URL (the system browser by default)
     */
    public function __construct(
        public readonly string $authorizeUrl,
        public readonly string $tokenUrl,
        public readonly string $clientId,
        public readonly string $redirectUri,
        public readonly array $scopes = [],
        public readonly array $parameters = [],
        public readonly string $storageKey = 'oauth.tokens',
        public readonly ?string $clientSecret = null,
        public readonly ?string $revokeUrl = null,
        private readonly ?Closure $openBrowser = null,
    ) {
        foreach (['authorizeUrl' => $authorizeUrl, 'tokenUrl' => $tokenUrl] as $name => $url) {
            if (!str_starts_with($url, 'https://') && !preg_match('#^http://(localhost|127\.0\.0\.1|\[::1\])(:\d+)?/#', $url)) {
                throw new InvalidArgumentException("OAuth {$name} must be https (or http on localhost).");
            }
        }
        if (!str_contains($redirectUri, ':')) {
            throw new InvalidArgumentException('The OAuth redirect URI needs a scheme (myapp://oauth/callback).');
        }
    }

    /**
     * Opens the provider's sign-in page; `$done` runs with the tokens once
     * the browser comes back and the code is exchanged.
     *
     * @param Closure(Tokens): void $done
     * @param (Closure(string): void)|null $failed the provider refused (access_denied…), the state did not match or the exchange failed
     */
    public function signIn(Closure $done, ?Closure $failed = null): void
    {
        $pkce = Pkce::create();
        $state = Pkce::state();
        $this->cancel();
        $this->claim = Linking::claim($this->redirectUri, function (string $url) use ($pkce, $state, $done, $failed): void {
            $this->cancel();
            parse_str((string) parse_url($url, PHP_URL_QUERY), $query);
            if (($query['state'] ?? null) !== $state) {
                $failed?->__invoke('The sign-in answer does not match the request (state).');

                return;
            }
            if (isset($query['error'])) {
                $failed?->__invoke((string) ($query['error_description'] ?? $query['error']));

                return;
            }
            if (!is_string($query['code'] ?? null)) {
                $failed?->__invoke('The provider sent no authorization code.');

                return;
            }
            $this->token([
                'grant_type' => 'authorization_code',
                'code' => $query['code'],
                'redirect_uri' => $this->redirectUri,
                'code_verifier' => $pkce->verifier,
            ], null, $done, $failed);
        });
        $url = $this->authorizeUrl.(str_contains($this->authorizeUrl, '?') ? '&' : '?').http_build_query([
            'response_type' => 'code',
            'client_id' => $this->clientId,
            'redirect_uri' => $this->redirectUri,
            'scope' => implode(' ', $this->scopes),
            'state' => $state,
            'code_challenge' => $pkce->challenge,
            'code_challenge_method' => 'S256',
        ] + $this->parameters, '', '&', PHP_QUERY_RFC3986);
        $this->openBrowser !== null ? ($this->openBrowser)($url) : Linking::open($url);
    }

    /** Stops waiting for the browser (the user gave up). */
    public function cancel(): void
    {
        if ($this->claim !== null) {
            Linking::release($this->claim);
            $this->claim = null;
        }
    }

    /**
     * The stored tokens, refreshed first when they expire within a minute.
     * Null when signed out or when the refresh is refused.
     *
     * @param Closure(?Tokens): void $done
     */
    public function tokens(Closure $done): void
    {
        $use = function (?Tokens $tokens) use ($done): void {
            if ($tokens === null || !$tokens->expired()) {
                $done($tokens);

                return;
            }
            if ($tokens->refreshToken === null) {
                $done(null);

                return;
            }
            $this->token(['grant_type' => 'refresh_token', 'refresh_token' => $tokens->refreshToken], $tokens, $done, static fn () => $done(null));
        };
        if ($this->cached !== null) {
            $use($this->cached);

            return;
        }
        SecureStorage::get($this->storageKey, function (?string $json) use ($use): void {
            $this->cached = $json === null ? null : Tokens::fromJson($json);
            $use($this->cached);
        }, static fn () => $use(null));
    }

    /** @param Closure(?string): void $done a valid access token, or null when signed out */
    public function accessToken(Closure $done): void
    {
        $this->tokens(static fn (?Tokens $tokens) => $done($tokens?->accessToken));
    }

    /** @param (Closure(): void)|null $done */
    public function signOut(?Closure $done = null): void
    {
        $tokens = $this->cached;
        $this->cached = null;
        $this->cancel();
        if ($this->revokeUrl !== null && ($tokens?->refreshToken ?? $tokens?->accessToken) !== null) {
            Http::post($this->revokeUrl, static function (): void {}, ['Content-Type' => 'application/x-www-form-urlencoded'], http_build_query([
                'token' => $tokens->refreshToken ?? $tokens->accessToken,
                'client_id' => $this->clientId,
            ]));
        }
        SecureStorage::delete($this->storageKey, static fn () => $done?->__invoke());
    }

    /**
     * @param array<string, string> $grant
     * @param Closure(Tokens): void $done
     * @param (Closure(string): void)|null $failed
     */
    private function token(array $grant, ?Tokens $previous, Closure $done, ?Closure $failed): void
    {
        $body = $grant + ['client_id' => $this->clientId] + ($this->clientSecret === null ? [] : ['client_secret' => $this->clientSecret]);
        Http::post($this->tokenUrl, function (HttpResponse $response) use ($previous, $done, $failed): void {
            $json = json_decode($response->body, true);
            $tokens = $response->successful() && is_array($json) ? Tokens::fromResponse($json, $previous) : null;
            if ($tokens === null) {
                $message = is_array($json) ? (string) ($json['error_description'] ?? $json['error'] ?? '') : '';
                $failed?->__invoke($message !== '' ? $message : "The token endpoint answered {$response->statusCode}.");

                return;
            }
            $this->cached = $tokens;
            SecureStorage::set($this->storageKey, $tokens->toJson(), static fn () => $done($tokens));
        }, ['Content-Type' => 'application/x-www-form-urlencoded', 'Accept' => 'application/json'], http_build_query($body, '', '&', PHP_QUERY_RFC3986));
    }
}
