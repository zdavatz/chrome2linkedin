# chrome2linkedin

Chrome extension that posts text to your LinkedIn feed via the official LinkedIn API.

## How it works

The extension is intentionally thin and talks to a small local helper service so the LinkedIn `client_secret` never lives inside a bundle that anyone who installs the extension could unpack.

```
[ extension popup ] --HTTP--> [ helper on 127.0.0.1:8093 ] --HTTPS--> api.linkedin.com
                                       ^
                                       reads ~/linkedin_token.json
                                            ~/linkedin_credentials.json
```

OAuth itself is delegated to the CLI tool in [`li_push_rs`](../old2new/li_push_rs) — both share the same token files in `$HOME`, so refreshing the token in one place keeps the other in sync.

## Setup

1. **One-time auth** (uses the sibling CLI):
   ```sh
   cd ~/.software/old2new/li_push_rs
   cargo run --release -- --auth
   ```
   This writes `~/linkedin_token.json`.

2. **Start the local helper**:
   ```sh
   cd helper
   cargo run --release
   ```
   Listens on `http://127.0.0.1:8093`. Override the port with `CHROME2LINKEDIN_PORT=…` (also update `extension/manifest.json` and `extension/popup.js`).

3. **Load the extension**:
   - Open `chrome://extensions`
   - Enable **Developer mode**
   - **Load unpacked** → select the `extension/` directory

Click the toolbar icon, type your post, pick visibility (Public / Connections only), submit. The popup shows the resulting post URL.
