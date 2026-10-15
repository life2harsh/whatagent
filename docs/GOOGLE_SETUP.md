# Google setup

The daemon talks to Google directly: Gemini for understanding messages, the Calendar API for events, and (optionally) Google Keep through the local `keep-bridge`.

## Gemini

Create an API key in Google AI Studio and set `GEMINI_API_KEY`. `GEMINI_MODEL` handles text; `GEMINI_AUDIO_MODEL` (optional) handles voice notes. Each request is one `generateContent` call with a JSON response schema, so the model can only return structured event data. Inline audio is limited to 20 MB, far above typical voice notes.

## Google Calendar (service account)

A service account signs its own short-lived tokens, so there is no OAuth consent screen and nothing that expires or needs reconnecting.

1. In Google Cloud Console, enable the **Google Calendar API** for your project.
2. IAM & Admin → Service Accounts → **Create service account** (no roles needed).
3. Open it → Keys → Add key → **JSON**. Pass the downloaded file to the installer with `--service-account` (see "Deploying to a VPS" in the README), or install it by hand, owner-readable only:

   ```bash
   sudo install -m 0600 -o wa-calendar -g wa-calendar service-account.json /opt/wa-calendar-agent/service-account.json
   ```

4. In Google Calendar → Settings → your calendar → **Share with specific people**, add the service account's `client_email` with **Make changes to events**.
5. The installer asks for the rest. For a manual setup, set `GOOGLE_SERVICE_ACCOUNT_KEY_PATH` to the key file and `GOOGLE_CALENDAR_ID` to the calendar ID (your Gmail address for your main calendar; other calendars show it under "Integrate calendar").

Events are created with an ID derived from the WhatsApp message ID, so a replayed or retried message can never create a duplicate. Service accounts cannot invite attendees on personal calendars; this agent does not need to.

## Google Keep (optional)

Google's official Keep API is limited to Workspace accounts, so `keep-bridge/` uses the unofficial [gkeepapi](https://github.com/kiwiz/gkeepapi) with a **master token**. Treat that token like your password: it grants broad access to the account, and Google may revoke it (for example after a password change), in which case notes queue up in SQLite until you replace it.

The quick way, from your machine (after the agent is installed):

```bash
deploy/keep-setup.sh you@your-server -i ~/.ssh/my-key
```

It opens a small sign-in window for you to sign in to Google, turns that into a master token locally, installs the bridge on the server, checks that it can log in, and connects the agent. Re-run it if Google revokes the token. To ship new bridge code without signing in again, use `deploy/keep-setup.sh you@your-server -i ~/.ssh/my-key --update`, which reuses the token already on the server. Each task becomes an item on one Keep checklist, named by `KEEP_LIST_TITLE` (default "WhatsApp Todos"). The manual steps follow.

1. Get a master token on a trusted machine:

   ```bash
   python3 -m venv --system-site-packages /tmp/gpsoauth
   /tmp/gpsoauth/bin/pip install gpsoauth pywebview
   /tmp/gpsoauth/bin/python keep-bridge/get_master_token.py
   ```

   It opens a small sign-in window (an embedded web view, which Google does not block the way it blocks a remote-controlled browser). Sign in and click "I agree", and it reads the one-time `oauth_token` cookie from the window itself, then prints `KEEP_EMAIL` and `KEEP_MASTER_TOKEN`. Without a graphical display, or with `--manual`, it asks you to copy the cookie from `https://accounts.google.com/EmbeddedSetup` by hand instead.

2. Install the bridge on the server:

   ```bash
   sudo install -d -o wa-calendar -g wa-calendar /opt/wa-calendar-agent/keep-bridge
   sudo install -m 0644 keep-bridge/keep_bridge.py keep-bridge/requirements.txt /opt/wa-calendar-agent/keep-bridge/
   sudo -u wa-calendar python3 -m venv /opt/wa-calendar-agent/keep-bridge/venv
   sudo -u wa-calendar /opt/wa-calendar-agent/keep-bridge/venv/bin/pip install -r /opt/wa-calendar-agent/keep-bridge/requirements.txt
   ```

3. Create `/etc/wa-calendar-keep-bridge.env` (mode `0600`):

   ```text
   KEEP_EMAIL=you@gmail.com
   KEEP_MASTER_TOKEN=aas_et/...
   KEEP_BRIDGE_TOKEN=<random string, e.g. openssl rand -hex 32>
   KEEP_BRIDGE_PORT=8765
   ```

4. Install `deploy/keep-bridge.service`, then set `KEEP_BRIDGE_URL=http://127.0.0.1:8765/notes` and the same `KEEP_BRIDGE_TOKEN` in the agent's environment.

The bridge listens only on localhost, requires the bearer token, and remembers which task IDs it has already turned into notes so retries don't duplicate them.
