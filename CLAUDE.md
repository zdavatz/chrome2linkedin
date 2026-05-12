# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

Chrome extension (Manifest V3) that posts text to the user's LinkedIn feed via the official LinkedIn `/rest/posts` API. The extension itself is intentionally thin — it talks to a local helper service that holds the OAuth credentials, so the LinkedIn `client_secret` never ends up inside a bundle that can be unpacked by anyone who installs the extension.

## Two-part architecture

```
[ Chrome extension popup ] --HTTP--> [ helper on 127.0.0.1:8093 ] --HTTPS--> [ api.linkedin.com ]
                                            ^
                                            └─ reads ~/.linkedin_credentials.json (manual)
                                                     ~/.linkedin_token.json     (written by `helper auth`)
```

- **`helper/`** — Rust crate (axum + tokio + reqwest). Single binary with two subcommands:
  - `auth` — runs the OAuth dance: opens the browser, listens on `127.0.0.1:8092/callback`, exchanges the code for a token, decodes the id_token JWT's `sub` claim to get the `person_id`, and writes `~/.linkedin_token.json`. Falls back to `GET /v2/userinfo` if the JWT path fails. Best-effort kills any stale listener on port 8092 via `lsof` (no-op on systems without it).
  - server mode (default, or `serve`):
    - `GET /status` — returns `{ has_token, person_id_present, refresh_token_present }`. The popup probes this on open and disables the submit button when the token is missing.
    - `POST /post` — body `{ commentary, visibility }` → calls LinkedIn `POST /rest/posts`, returns `{ post_id, post_url }`. On a 401 from LinkedIn it transparently refreshes the access token using the saved `refresh_token` + credentials and retries once.
  - Constants worth knowing: `LINKEDIN_VERSION = "202603"` (LinkedIn versioned API header), `DEFAULT_PORT = 8093` (overridable via `CHROME2LINKEDIN_PORT`), `AUTH_CALLBACK_PORT = 8092` (must match an Authorized redirect URL configured on the LinkedIn app).
- **`extension/`** — MV3 extension. `manifest.json` declares only `storage` permission plus `host_permissions` for `http://localhost:8093/*`. `popup.js` persists the textarea draft and visibility choice in `chrome.storage.local` so reopening the popup restores in-progress text.

## Token files

Two JSON files in `$HOME`:

- `.linkedin_credentials.json` — `{ client_id, client_secret }`. **Created manually** by the user from the LinkedIn Developer App's Auth tab. Never written by the helper.
- `.linkedin_token.json` — `{ access_token, refresh_token, person_id, expires_in }`. Written by `helper auth`; rewritten in place on every successful refresh.

Posting author URN format: `urn:li:person:<person_id>`. If LinkedIn bumps the API version, update `LINKEDIN_VERSION` in `helper/src/main.rs`.

## Build & run

```sh
# 1. One-time: write ~/.linkedin_credentials.json yourself, then:
cd helper && cargo run --release -- auth

# 2. Start the local helper
cargo run --release

# 3. Load the extension
#    chrome://extensions → Developer mode → "Load unpacked" → select extension/
```

The helper listens on `http://127.0.0.1:8093`. Override with `CHROME2LINKEDIN_PORT=…` — if you do, also update `HELPER` in `extension/popup.js` AND the `host_permissions` entry in `extension/manifest.json` (they must match or the popup's fetch will be blocked by Chrome).

## Things that will bite future-you

- **Manifest V3 host_permissions are the gate, not CORS.** The helper sets `CorsLayer::permissive()`, but the popup can still be blocked if the localhost URL isn't in `host_permissions`. Keep the port in `manifest.json` and `popup.js` aligned.
- **`person_id` is derived from the OAuth id_token JWT (`sub` claim).** If it's empty in the token file, the helper returns 412 and refuses to post. Re-run `helper auth`.
- **LinkedIn returns the new post's URN in the `x-restli-id` response header**, not the body. The helper builds the post URL from that header.
- **`invalid_client` on token exchange** ≠ `client_id` is wrong. The browser consent step doesn't verify the secret — only the subsequent POST to `/oauth/v2/accessToken` does. If you see `401 invalid_client`, regenerate the Primary Client Secret in the LinkedIn dashboard and copy via the UI's copy button (not visually).
- **The OAuth callback port (8092) is separate from the helper port (8093).** Both must be free during `auth`; only 8093 needs to be free for normal operation. 8092 must match a redirect URL registered on the LinkedIn app.
- **`commentary` uses LinkedIn's "Little Text" format.** `( ) < > @ | { } [ ] * _ ~ \` are control characters and **silently truncate** the post at the first unescaped occurrence — there's no error, the post just publishes with everything from that char onward missing. `escape_little_text()` in `helper/src/main.rs` backslash-escapes them before sending. If you add a new client (CLI, etc.) that bypasses the helper, you must do the same escaping yourself.
- **`refresh_token` is often absent.** "Sign In with LinkedIn using OpenID Connect" Standard Tier does not include refresh tokens by default — only the access token (valid ~60 days). The helper attempts a refresh on 401 but gracefully fails back to "re-run `helper auth`". To get refresh tokens, the app needs the separate "Refresh Token" product enabled in the LinkedIn developer dashboard.
