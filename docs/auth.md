# Sign-in with OAuth

`Pam\Desktop\Auth\OAuthClient` signs users in with OAuth 2.0 and OpenID Connect the way native apps should (RFC 8252). The system browser opens the provider's page, and the provider sends the browser back to the app's own URL scheme. The app then exchanges the code using PKCE. Tokens are kept in the system keychain and refreshed before they expire. It ships inside `pushinbr/pam-desktop`.

```php
use Pam\Desktop\Auth\OAuthClient;
use Pam\Desktop\Auth\Tokens;

$this->auth = new OAuthClient(
    authorizeUrl: 'https://accounts.example.com/oauth/authorize',
    tokenUrl: 'https://accounts.example.com/oauth/token',
    clientId: 'desktop-app',
    redirectUri: 'myapp://oauth/callback',
    scopes: ['openid', 'profile', 'email', 'offline_access'],
);

$this->auth->signIn(function (Tokens $tokens): void {
    $this->user = $tokens->claims()['name'] ?? null;
}, function (string $error): void {
    $this->error = $error;
});
```

## Setting it up

1. Register a *native* or *public* client with the provider and use a redirect URI on your own scheme, such as `myapp://oauth/callback`.
2. List the scheme in `pam-desktop.json`: `"urlSchemes": ["myapp"]`. The installers register it with the system, and `pam-desktop dev` registers it while it runs. With a scheme, `run` keeps a single instance, so the browser's redirect reaches the window that is already open (see [Distribution](distribution.md#links-and-single-instance)).

## Using the tokens

```php
$this->auth->accessToken(function (?string $token): void {
    Http::get('https://api.example.com/me', $this->show(...), ['Authorization' => "Bearer {$token}"]);
});
```

- `tokens()` and `accessToken()` read the keychain once, then memory. When the access token expires within a minute they refresh it first, keeping the refresh and id tokens the provider did not replace, and pass `null` when the user is signed out or the refresh is refused.
- `Tokens` has `accessToken`, `refreshToken`, `idToken`, `expiresAt`, `tokenType`, `scope`, `header()` (`Bearer …`) and `expired($margin)`.
- `claims()` reads the id token's claims (`name`, `email`, `sub`…). They are read but not verified, so verify the token on your server before you trust them.
- `signOut()` deletes the stored tokens, and also revokes them when a `revokeUrl` was given. `cancel()` stops waiting for the browser.

## Options

| Argument | Meaning |
| --- | --- |
| `parameters` | Extra authorize parameters: `audience`, `prompt`, `login_hint`… |
| `storageKey` | The keychain entry (`oauth.tokens` by default). Use one per account or provider. |
| `clientSecret` | Only for providers that require one even for native apps. A secret shipped in an app is not secret. |
| `revokeUrl` | The token revocation endpoint, called by `signOut()`. |
| `openBrowser` | Opens the authorize URL yourself (the system browser by default). |

The endpoints must be `https`, or `http` on `localhost` while developing. The answer is accepted only when its `state` matches the request, and the code only counts with the PKCE verifier of that same request.

## Links that belong to a library

`OAuthClient` waits for its redirect with `Linking::claim($prefix, $handler)`. A claim takes the incoming links that start with its prefix before the application's own `Linking::listen()` listener sees any, and `Linking::release($id)` ends it. Other libraries that receive callbacks on the app's scheme can do the same.
