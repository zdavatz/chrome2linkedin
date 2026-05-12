# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project

Chrome extension (Manifest V3) that posts text to the user's LinkedIn feed via the official LinkedIn `/rest/posts` API. The extension itself is intentionally thin — it talks to a local helper service that holds the OAuth credentials, so the LinkedIn `client_secret` never ends up inside a bundle that can be unpacked by anyone who installs the extension.

## Two-part architecture

```
[ Chrome extension popup ] --HTTP--> [ helper on 127.0.0.1:8093 ] --HTTPS--> [ api.linkedin.com ]
                                            ^
                                            └─ reads ~/linkedin_token.json + ~/linkedin_credentials.json
                                               (both files are produced by `li_push --auth` from
                                                ../old2new/li_push_rs)
```

- **`helper/`** — Rust crate (axum + tokio + reqwest). Exposes:
  - `GET /status` — returns `{ has_token, person_id_present, refresh_token_present }`. The popup probes this on open and disables the submit button when the token is missing.
  - `POST /post` — body `{ commentary, visibility }` → calls LinkedIn `POST /rest/posts`, returns `{ post_id, post_url }`. On a 401 from LinkedIn it transparently refreshes the access token using the saved `refresh_token` + credentials and retries once.
  - Constants worth knowing: `LINKEDIN_VERSION = "202603"` (LinkedIn versioned API header), `DEFAULT_PORT = 8093` (overridable via `CHROME2LINKEDIN_PORT`). The helper does NOT do OAuth itself — initial auth happens in `li_push_rs` via `li_push --auth`.
- **`extension/`** — MV3 extension. `manifest.json` declares only `storage` permission plus `host_permissions` for `http://localhost:8093/*`. `popup.js` persists the textarea draft and visibility choice in `chrome.storage.local` so reopening the popup restores in-progress text.

## Relationship to li_push_rs

The sibling project `~/.software/old2new/li_push_rs` is the source of truth for OAuth. It writes two files into `$HOME`:

- `linkedin_credentials.json` — `{ client_id, client_secret }` (manually created by the user).
- `linkedin_token.json` — `{ access_token, refresh_token, person_id, expires_in }` (written by `li_push --auth`).

Both this helper and the Rust CLI use the same `LINKEDIN_VERSION` and the same person-URN-based author format (`urn:li:person:<person_id>`). If LinkedIn bumps the API version, update it in both projects.

When the helper needs to refresh, it reuses the same OAuth refresh flow li_push_rs uses (`grant_type=refresh_token` against `https://www.linkedin.com/oauth/v2/accessToken`) and writes the rotated token back to the same `linkedin_token.json` file — so the CLI and the extension stay in sync.

## Build & run

```sh
# 1. One-time: get a token (from the other repo)
cd ~/.software/old2new/li_push_rs && cargo run --release -- --auth

# 2. Start the local helper (this repo)
cd ~/.software/chrome2linkedin/helper && cargo run --release

# 3. Load the extension
#    chrome://extensions → Developer mode → "Load unpacked" → select extension/
```

The helper listens on `http://127.0.0.1:8093`. Override with `CHROME2LINKEDIN_PORT=…` — if you do, also update `HELPER` in `extension/popup.js` AND the `host_permissions` entry in `extension/manifest.json` (they must match or the popup's fetch will be blocked by Chrome).

## Things that will bite future-you

- **Manifest V3 host_permissions are the gate, not CORS.** The helper sets `CorsLayer::permissive()`, but the popup can still be blocked if the localhost URL isn't in `host_permissions`. Keep the port in `manifest.json` and `popup.js` aligned.
- **`person_id` is derived from the OAuth id_token JWT (`sub` claim).** If it's empty in the token file, the helper returns 412 and refuses to post. Re-run `li_push --auth`.
- **LinkedIn returns the new post's URN in the `x-restli-id` response header**, not the body. The helper builds the post URL from that header.
