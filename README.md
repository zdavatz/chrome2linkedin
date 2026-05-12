# chrome2linkedin

Chrome extension that posts text to your LinkedIn feed via the official LinkedIn API.

## How it works

The extension is intentionally thin and talks to a small local helper service so the LinkedIn `client_secret` never lives inside a bundle that anyone who installs the extension could unpack.

```
[ extension popup ] --HTTP--> [ helper on 127.0.0.1:8093 ] --HTTPS--> api.linkedin.com
                                       ^
                                       reads ~/.linkedin_credentials.json
                                            ~/.linkedin_token.json
```

The helper handles both the one-time OAuth flow and the runtime posting + token refresh — this repo has no external dependencies beyond a LinkedIn Developer App.

## Setup

1. **Create a LinkedIn Developer App** at https://www.linkedin.com/developers/apps
   - **Auth** tab → add `http://localhost:8092/callback` to **Authorized redirect URLs**.
   - **Products** tab → request both **Sign In with LinkedIn using OpenID Connect** and **Share on LinkedIn**.
   - Copy **Client ID** and **Primary Client Secret**.

2. **Save credentials** to `~/.linkedin_credentials.json`:
   ```json
   { "client_id": "...", "client_secret": "..." }
   ```

3. **Run the OAuth flow** to mint a token:
   ```sh
   cd helper
   cargo run --release -- auth
   ```
   Opens your browser, catches the callback on port 8092, writes `~/.linkedin_token.json`.

4. **Start the helper**:
   ```sh
   cargo run --release
   ```
   Listens on `http://127.0.0.1:8093`. Override the port with `CHROME2LINKEDIN_PORT=…` (also update `extension/manifest.json` and `extension/popup.js`).

5. **Load the extension**:
   - Open `chrome://extensions`
   - Enable **Developer mode**
   - **Load unpacked** → select the `extension/` directory

Click the toolbar icon, type your post, pick visibility (Public / Connections only), submit. The popup shows the resulting post URL.

The helper escapes LinkedIn's "Little Text" control characters (`( ) < > @ | { } [ ] * _ ~ \`) in your post before sending, so things like `(38:15)` render literally instead of silently truncating the post.

Access tokens last ~60 days. If your LinkedIn app has the Refresh Token product enabled, the helper auto-refreshes on 401. Otherwise (the default for Sign-In-with-LinkedIn Standard Tier), re-run `cargo run --release -- auth` when the token expires.
