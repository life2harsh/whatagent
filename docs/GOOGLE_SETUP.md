# Google setup

The daemon talks to Google directly: Gemini for understanding messages, and optionally the Calendar API for events and Google Keep (through the local `keep-bridge`). Only Gemini is required.

## The quick way: Cloud Shell

Open [Google Cloud Shell](https://shell.cloud.google.com/?show=terminal) and run:

```bash
curl -fsSL https://raw.githubusercontent.com/life2harsh/whatagent/main/deploy/google-setup.sh | bash
```

It creates (or reuses) a Google Cloud project named "whatagent", turns on the Calendar and Gemini APIs, creates a service account, and prints a Gemini API key and a calendar code for the installer. The calendar code is `cal1:` followed by the calendar ID and the service account key, gzipped and base64-encoded, so treat it like a password. The only step left is sharing your calendar with the service account, which the script spells out. Add `--calendar-id ID` to use a calendar other than your main one. Running it again reuses everything and makes a fresh code.

The rest of this page is the manual route.

## Gemini

Create an API key in Google AI Studio and set `GEMINI_API_KEY`. `GEMINI_MODEL` handles text; `GEMINI_AUDIO_MODEL` (optional) handles voice notes. Each request is one `generateContent` call with a JSON response schema, so the model can only return structured event data. Inline audio is limited to 20 MB, far above typical voice notes.

## Google Calendar (optional, service account)

Without Google Calendar, an event becomes a todo due when it starts, with a WhatsApp reminder 30 minutes before (a repeating event only gets its first reminder). With it, events go into your real calendar.

A service account signs its own short-lived tokens, so there is no OAuth consent screen and nothing that expires or needs reconnecting. It needs no IAM roles: you share your calendar with it, the same way you would with a person.

1. In Google Cloud Console, enable the **Google Calendar API** for your project.
2. IAM & Admin > Service Accounts > **Create service account** (skip the roles step).
3. Open it > Keys > Add key > **JSON**. Pass the downloaded file to the installer with `--service-account` (see "setting it up" in the README), or install it by hand, owner-readable only:

   ```bash
   sudo install -m 0600 -o wa-calendar -g wa-calendar service-account.json /opt/wa-calendar-agent/service-account.json
   ```

4. In Google Calendar > Settings > your calendar > **Share with specific people**, add the service account's `client_email` with **Make changes to events**.
5. The installer asks for the rest. For a manual setup, set `GOOGLE_SERVICE_ACCOUNT_KEY_PATH` to the key file and `GOOGLE_CALENDAR_ID` to the calendar ID (your Gmail address for your main calendar; other calendars show it under "Integrate calendar").

Events are created with an ID derived from the WhatsApp message ID, so a replayed or retried message can never create a duplicate. Service accounts cannot invite attendees on personal calendars; this agent does not need to.

## Google Keep (optional)

Google's official Keep API is limited to Workspace accounts, so `keep-bridge/` uses the unofficial [gkeepapi](https://github.com/kiwiz/gkeepapi) with a **master token**. Treat that token like your password: it grants broad access to the account, and Google may revoke it (for example after a password change), in which case notes queue up in SQLite until you replace it.

The installer asks whether to connect it (or run it again with `--keep`). It then asks for a **Keep code**, which you make on your own computer, not the server:

- **Windows:** download `keep-signin-windows.exe` from the latest release and open it.
- **Mac or Linux:** `curl -fsSL https://raw.githubusercontent.com/life2harsh/whatagent/main/deploy/keep-signin.sh | bash`

A small window opens (an embedded web view, which Google doesn't block the way it blocks a remote-controlled browser). Enter your Gmail address, sign in, and click "I agree". The helper reads the one-time `oauth_token` cookie from the window itself, exchanges it for a master token, and shows the Keep code (`keep1:` followed by the email and token, base64-encoded). Paste that into the installer. It installs the bridge, checks that it can log in, and connects the agent. Without a graphical display, or with `--manual`, the helper asks you to copy the cookie from `https://accounts.google.com/EmbeddedSetup` by hand instead.

If Google revokes the token, run the installer with `--keep` again. Each task becomes an item on one Keep checklist, named by `KEEP_LIST_TITLE` (default "WhatsApp Todos").

From a Linux desktop with this repo checked out, `deploy/keep-setup.sh you@your-server -i ~/.ssh/my-key` does the whole thing over SSH with no code to copy, and `--update` ships new bridge code without signing in again. The manual steps follow.

1. Get a master token on a trusted machine:

   ```bash
   python3 -m venv --system-site-packages /tmp/gpsoauth
   /tmp/gpsoauth/bin/pip install gpsoauth pywebview
   /tmp/gpsoauth/bin/python keep-bridge/get_master_token.py
   ```

   Add `--env` to print `KEEP_EMAIL` and `KEEP_MASTER_TOKEN` instead of a Keep code.

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
