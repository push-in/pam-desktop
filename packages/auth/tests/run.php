<?php

declare(strict_types=1);

use Pam\Desktop\Auth\OAuthClient;
use Pam\Desktop\Auth\Pkce;
use Pam\Desktop\Auth\Tokens;

require dirname(__DIR__, 3).'/vendor/autoload.php';

$fail = static fn (string $message) => throw new RuntimeException($message);

// S256: base64url(sha256(verifier)), checked against Python's hashlib.
Pkce::fromVerifier('dBjftJeZ4CVP-mJ92IZOXVvQe1LYbGeNCQ6Q8mPR2jA')->challenge === 'FY7iouik2qgGBefEcN7O0xZRWIjQXh13OHAACjlCjq4' || $fail('PKCE S256 challenge.');
$pkce = Pkce::create();
preg_match('/^[A-Za-z0-9_-]{43,128}$/', $pkce->verifier) === 1 && $pkce->verifier !== Pkce::create()->verifier || $fail('Verifiers are random and well formed.');

$idToken = 'h.'.Pkce::base64Url(json_encode(['name' => 'Ana', 'email' => 'ana@example.com'])).'.s';
$tokens = Tokens::fromResponse(['access_token' => 'a1', 'refresh_token' => 'r1', 'expires_in' => 3600, 'token_type' => 'bearer', 'id_token' => $idToken], null, 1_000);
$tokens?->expiresAt === 4_600 && $tokens->header() === 'Bearer a1' || $fail('Tokens from a token response.');
!$tokens->expired(60, 4_000) && $tokens->expired(60, 4_550) || $fail('Expiry counts a margin.');
$tokens->claims()['email'] === 'ana@example.com' || $fail('id token claims.');
$refreshed = Tokens::fromResponse(['access_token' => 'a2', 'expires_in' => 60], $tokens, 5_000);
$refreshed?->refreshToken === 'r1' && $refreshed->idToken === $idToken || $fail('A refresh keeps the refresh and id tokens it does not replace.');
Tokens::fromJson($tokens->toJson()) == $tokens || $fail('Tokens round-trip through storage.');
Tokens::fromResponse(['error' => 'invalid_grant']) === null && Tokens::fromJson('{') === null || $fail('Bad answers give no tokens.');

foreach ([
    ['http://accounts.example.com/authorize', 'https://a/token', 'app://cb'],
    ['https://a/authorize', 'https://a/token', 'no-scheme'],
] as [$authorize, $token, $redirect]) {
    try {
        new OAuthClient($authorize, $token, 'id', $redirect);
        $fail('Insecure endpoints and redirect URIs without a scheme are refused.');
    } catch (InvalidArgumentException) {
    }
}
new OAuthClient('http://127.0.0.1:8080/authorize', 'http://localhost/token', 'id', 'app://cb');
echo "Auth: PKCE, tokens, expiry, claims, storage form and endpoint checks: passed\n";
