# wa-calendar-agent

A small Rust daemon for turning text messages and voice notes sent to a WhatsApp self-chat into Google Calendar events, with tasks, reminders, and optional Google Keep sync.

## Scope

Only a configured owner self-chat is eligible for processing. The adapter uses `whatsapp-rust` 0.7 with persistent SQLite session storage. Messages and voice notes are parsed by Gemini, which decides whether the request is a calendar event or a task, then it is validated in Rust and either created in Google Calendar through a service account or stored as a task. A voice note like "remind me to buy milk tomorrow at 6pm" becomes a task (and, when Keep sync is on, a Keep note); one like "dentist tomorrow at 5" becomes a calendar event. Voice notes are downloaded and decrypted by the WhatsApp client and sent to Gemini inline.

## Local development

1. Copy `.env.example` to `.env` and fill every required value.
2. Set up Gemini and the Google Calendar service account as described in `docs/GOOGLE_SETUP.md`.
3. Run the command below.

```bash
cargo run -- test-calendar "cal OSDC meeting tomorrow at 6pm"
```

The command writes state to `DATABASE_URL`, calls Gemini and Google Calendar, and prints the reply the WhatsApp adapter would send to the self-chat. It creates a real event.

To pair and start the daemon:

```bash
cargo run -- run
```

The QR code is printed to standard output only during initial pairing. `WHATSAPP_SESSION_DATABASE_PATH` persists the linked-device state so it is reused on restart. Treat that file as a credential and keep it owner-readable only.

To link WhatsApp and find your `OWNER_JID`, set only `WHATSAPP_SESSION_DATABASE_PATH` and run the command below. After scanning the QR code, send yourself any message; the command prints the exact `OWNER_JID=` line to use and exits.

```bash
cargo run -- pair
```

`cargo run -- check` verifies the Gemini key and Google Calendar access without sending anything.

Run checks with:

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
cargo build --release
```

## Environment

See `.env.example`. `OWNER_JID` is the canonical WhatsApp JID for the account running the daemon. `GEMINI_API_KEY`, `GOOGLE_SERVICE_ACCOUNT_KEY_PATH`, and `GOOGLE_CALENDAR_ID` are required; `KEEP_BRIDGE_URL`/`KEEP_BRIDGE_TOKEN` enable Keep sync. `WHATSAPP_SESSION_DATABASE_PATH` is separate from the small agent-state database. Do not commit `.env`, the session database, or key files.

## State

SQLite stores message processing state, pending confirmations, tasks, and delivery outboxes. It is not a message archive. A message ID is claimed before any Google call, preventing replayed WhatsApp events from being processed twice. Calendar events are also created with an ID derived from the message ID, so even a lost response followed by a retry cannot create a duplicate.

## Tasks and WhatsApp reminders

Tasks and reminders are stored durably in the same SQLite database. You can create a task in two ways: the explicit `todo` command below (ISO-8601 times with a timezone offset), or plain language in a text or voice message, which Gemini reads as a task and turns into one with a due or reminder time.

```text
todo submit DBMS assignment | due 2026-09-18T17:00:00+05:30 | remind 2026-09-18T16:00:00+05:30 | note upload the PDF
tasks
done 1
remind 2026-09-18T16:00:00+05:30 | call Abhinav
```

The connected daemon checks due reminders every `REMINDER_POLL_SECONDS` seconds, sends the message to the configured self-chat, and records delivery in SQLite. A failed send remains recoverable instead of being silently discarded. When `KEEP_BRIDGE_URL` is configured, each new task is appended as an unchecked item to a single Google Keep checklist (named by `KEEP_LIST_TITLE`, default "WhatsApp Todos") through the local `keep-bridge`, so all your todos live in one list. Keep sync is durable and retried independently from task creation. After changing the bridge, redeploy it with `deploy/keep-setup.sh you@server -i key --update` (no new sign-in).

## Calendar details, confirmation, and recurrence

Agenda text, notes, and other supplied event details become the Google Calendar description. Explicit recurring requests are supported through validated RFC 5545 rules. For example, `cal add a recurring event every Wednesday at 3pm to solve the NPTEL assignment` becomes a weekly Wednesday event. Vague repetition is rejected rather than silently creating an incorrect series. When the time itself is vague, the agent replies with its best reading and waits for `yes` or `no`. Messages that are clearly not calendar requests are declined.

## Deploying to a VPS

Any Linux server with systemd works, including the cheapest VPS tier. Prebuilt binaries cover x86_64 and ARM64 with glibc 2.35+ (Ubuntu 22.04+, Debian 12+).

Before you start, you need:

- a Gemini API key from [Google AI Studio](https://aistudio.google.com/apikey);
- a Google service account JSON key (see `docs/GOOGLE_SETUP.md`; it takes about two minutes).

### One command on the server

Copy the key to the server, then run the installer there:

```bash
scp service-account.json you@your-server:
ssh you@your-server
curl -fsSL https://raw.githubusercontent.com/life2harsh/whatagent/main/deploy/install.sh | sudo bash -s -- --service-account ~/service-account.json
```

The installer:

1. downloads the latest release for your CPU and checks its checksum;
2. creates the `wa-calendar` user and `/opt/wa-calendar-agent`, and installs the systemd service;
3. asks for your Gemini key (it checks the key works), calendar ID and timezone;
4. shows who to share your calendar with, then tests Gemini and Calendar access;
5. prints a QR code: link it under WhatsApp > Settings > Linked devices;
6. asks you to send yourself one message, which tells it your `OWNER_JID`;
7. starts the service and waits until it's connected.

You can delete `~/service-account.json` afterwards; the installer keeps its own copy, readable only by the service.

### Or from your own machine

`deploy/deploy.sh` does the same over SSH, uploading the key for you:

```bash
deploy/deploy.sh you@your-server --service-account ./service-account.json
deploy/deploy.sh you@your-server -i ~/.ssh/my-key     # upgrade later
```

Add `--binary path/to/wa-calendar-agent` to deploy a binary you built yourself instead of a release.

### Upgrading and changing settings

Re-run the same command to upgrade. Settings, the WhatsApp session and your tasks are kept, the previous binary is backed up next to the new one, and the rollback command is printed at the end. Other options:

| Option | Effect |
| --- | --- |
| `--version v0.2.0` | Install a specific release instead of the latest. |
| `--reconfigure` | Ask for the Gemini key, calendar, timezone and service account again. |
| `--relink` | Unlink and show a new WhatsApp QR code. |

Settings live in `/etc/wa-calendar-agent.env` (after editing it, run `sudo systemctl restart wa-calendar-agent`). Useful commands:

```bash
journalctl -u wa-calendar-agent -f                         # logs
sudo systemctl restart wa-calendar-agent                   # restart
sudo -u wa-calendar sh -c 'set -a; . /etc/wa-calendar-agent.env; /opt/wa-calendar-agent/wa-calendar-agent check'
```

The service is capped at 25% CPU and 512 MB of memory, so it can share a small VPS with other services. For Keep sync, run `deploy/keep-setup.sh you@your-server` from your machine (see `docs/GOOGLE_SETUP.md`).

### Publishing releases

The installer downloads binaries from this repository's GitHub releases, so the repository must be public. Push a version tag to build and publish them:

```bash
git tag v0.1.0
git push origin v0.1.0
```

`.github/workflows/release.yml` builds x86_64 and ARM64 binaries and attaches them to the release. Forks can point the installer at their own releases with `WA_REPO=owner/repo`.

## Full-pipeline checklist

- [ ] The service account's email has "Make changes to events" on `GOOGLE_CALENDAR_ID`.
- [ ] `cargo run -- test-calendar "cal test event tomorrow at 6pm"` returns a created result.
- [ ] A deliberately vague request asks for confirmation, and `yes` creates exactly one Calendar event.
- [ ] Set `OWNER_JID` to the `OWNER_JID=` line printed by `wa-calendar-agent pair` (the installer does this for you).
- [ ] Run `cargo run -- run`, scan the QR code, and verify a session database is created with restrictive permissions.
- [ ] Send `cal test event tomorrow at 6pm` from the linked account's self-chat and receive one concise reply.
- [ ] Send a real voice note for an event and verify one Calendar event and one WhatsApp confirmation.
- [ ] Send a voice note like "remind me to buy milk tomorrow at 6pm" and verify one task (and a new item on the Keep checklist when sync is on).
- [ ] Send a clear recurring request and verify the event shows as repeating in Google Calendar.
- [ ] Add a task, list it, complete it, and verify one scheduled reminder reaches the self-chat.
