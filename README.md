# wa-calendar-agent

A small Rust daemon for turning text messages and voice notes sent to a WhatsApp self-chat into Google Calendar events, with tasks, reminders, and optional Google Keep sync.

## Scope

Only a configured owner self-chat is eligible for processing. The adapter uses `whatsapp-rust` 0.7 with persistent SQLite session storage. Messages and voice notes are parsed by Gemini into a structured event, validated in Rust, and created in Google Calendar through a service account. Voice notes are downloaded and decrypted by the WhatsApp client and sent to Gemini inline.

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

After pairing, the startup log prints both the account phone-number JID and LID JID. Start with the phone-number JID for `OWNER_JID`, then send a self-chat test. If it is ignored, use the LID shown in the same connection log and restart.

To pair and discover the two JIDs before configuring Google, set only `WHATSAPP_SESSION_DATABASE_PATH` and run:

```bash
cargo run -- pair
```

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

Tasks and reminders are stored durably in the same SQLite database. Use ISO-8601 times with a timezone offset until the natural-language task workflow is enabled.

```text
todo submit DBMS assignment | due 2026-09-18T17:00:00+05:30 | remind 2026-09-18T16:00:00+05:30 | note upload the PDF
tasks
done 1
remind 2026-09-18T16:00:00+05:30 | call Abhinav
```

The connected daemon checks due reminders every `REMINDER_POLL_SECONDS` seconds, sends the message to the configured self-chat, and records delivery in SQLite. A failed send remains recoverable instead of being silently discarded. When `KEEP_BRIDGE_URL` is configured, each new task is also written to Google Keep through the local `keep-bridge`. Keep sync is durable and retried independently from task creation.

## Calendar details, confirmation, and recurrence

Agenda text, notes, and other supplied event details become the Google Calendar description. Explicit recurring requests are supported through validated RFC 5545 rules. For example, `cal add a recurring event every Wednesday at 3pm to solve the NPTEL assignment` becomes a weekly Wednesday event. Vague repetition is rejected rather than silently creating an incorrect series. When the time itself is vague, the agent replies with its best reading and waits for `yes` or `no`. Messages that are clearly not calendar requests are declined.

## VPS installation

```bash
sudo useradd --system --home /opt/wa-calendar-agent --shell /usr/sbin/nologin wa-calendar
sudo install -d -o wa-calendar -g wa-calendar /opt/wa-calendar-agent
sudo install -m 0755 target/release/wa-calendar-agent /opt/wa-calendar-agent/wa-calendar-agent
sudo install -m 0600 -o wa-calendar -g wa-calendar .env /etc/wa-calendar-agent.env
sudo install -m 0644 deploy/wa-calendar-agent.service /etc/systemd/system/wa-calendar-agent.service
sudo systemctl daemon-reload
sudo systemctl enable --now wa-calendar-agent
```

The included service is deliberately capped at 25% CPU and 512 MB memory so it remains a lightweight tenant on the VPS. Build releases with a separate CPU/memory-capped build service rather than compiling unrestricted on the server.

Install the service account key as `/opt/wa-calendar-agent/service-account.json` (mode `0600`, owned by `wa-calendar`). For Keep sync, also install `deploy/keep-bridge.service` as described in `docs/GOOGLE_SETUP.md`.

## Full-pipeline checklist

- [ ] The service account's email has "Make changes to events" on `GOOGLE_CALENDAR_ID`.
- [ ] `cargo run -- test-calendar "cal test event tomorrow at 6pm"` returns a created result.
- [ ] A deliberately vague request asks for confirmation, and `yes` creates exactly one Calendar event.
- [ ] Set `OWNER_JID` to the exact canonical JID shown by WhatsApp pairing diagnostics.
- [ ] Run `cargo run -- run`, scan the QR code, and verify a session database is created with restrictive permissions.
- [ ] Send `cal test event tomorrow at 6pm` from the linked account's self-chat and receive one concise reply.
- [ ] Send a real voice note and verify one Calendar event and one WhatsApp confirmation.
- [ ] Send a clear recurring request and verify the event shows as repeating in Google Calendar.
- [ ] Add a task, list it, complete it, and verify one scheduled reminder reaches the self-chat.
