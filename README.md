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

The popup has two tabs:

- **Compose** — write a post, pick visibility (Public / Connections only), submit. The popup shows the resulting post URL.
- **Recent** — list of posts you've made through this extension, with **Edit** and **Delete** buttons per post. Delete uses a two-click inline confirm.

The Recent tab reads from a local log at `~/.linkedin_post_log.json` (written by the helper on every successful post). LinkedIn's "list my posts" API endpoint is gated behind a higher product tier than Share-on-LinkedIn Default, so we keep our own log rather than query LinkedIn. Pre-existing posts created outside this extension don't appear in Recent.

The helper escapes LinkedIn's "Little Text" control characters (`( ) < > @ | { } [ ] * _ ~ \`) in your post before sending, so things like `(38:15)` render literally instead of silently truncating the post.

Access tokens last ~60 days. If your LinkedIn app has the Refresh Token product enabled, the helper auto-refreshes on 401. Otherwise (the default for Sign-In-with-LinkedIn Standard Tier), re-run `cargo run --release -- auth` when the token expires.

If the helper isn't running when you open the popup, the Compose view shows a copy-paste `nohup …` snippet to start it in the background. The path adapts to your OS (`~/software/...` on macOS, `~/.software/...` on Linux); if you cloned the repo elsewhere, edit the snippet in `extension/popup.js`.
