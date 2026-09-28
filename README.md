# wa-calendar-agent

A small Rust daemon for turning text messages and voice notes sent to a WhatsApp self-chat into Google Calendar events through n8n.

## Scope

Only a configured owner self-chat is eligible for processing. The adapter uses `whatsapp-rust` 0.7 with persistent SQLite session storage. Voice notes are downloaded and decrypted by the WhatsApp client, then sent as multipart data to a separate authenticated n8n transcription webhook.

## Local development

1. Copy `.env.example` to `.env` and fill every required value.
2. Start an n8n workflow exposing the two webhook endpoints documented in `docs/N8N_SETUP.md`.
3. Run the command below.

```bash
cargo run -- test-calendar "cal OSDC meeting tomorrow at 6pm"
```

The command writes state to `DATABASE_URL`, posts a typed request to n8n, and prints the concise response that the future WhatsApp adapter will send to the self-chat.

To pair and start the daemon:

```bash
cargo run -- run
```

The QR code is printed to standard output only during initial pairing. `WHATSAPP_SESSION_DATABASE_PATH` persists the linked-device state so it is reused on restart. Treat that file as a credential and keep it owner-readable only.

After pairing, the startup log prints both the account phone-number JID and LID JID. Start with the phone-number JID for `OWNER_JID`, then send a self-chat test. If it is ignored, use the LID shown in the same connection log and restart.

To pair and discover the two JIDs before configuring n8n, set only `WHATSAPP_SESSION_DATABASE_PATH` and run:

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

`OWNER_JID` is the canonical WhatsApp JID for the account running the daemon. n8n URLs are formed by joining `N8N_BASE_URL` with their respective webhook paths. `N8N_WEBHOOK_SECRET` is sent as a bearer token on every webhook request. `N8N_AUDIO_WEBHOOK_PATH` is the separate audio-transcription route. `WHATSAPP_SESSION_DATABASE_PATH` is separate from the small agent-state database. Do not commit `.env`.

## State

SQLite stores only message processing state and pending confirmations. It is not a message archive. A message ID is claimed before sending to n8n, preventing replayed WhatsApp events from creating a duplicate request. n8n must enforce the same idempotency key before creating the calendar event because a lost HTTP response can still follow a successful event creation.

## Tasks and WhatsApp reminders

Tasks and reminders are stored durably in the same SQLite database. Use ISO-8601 times with a timezone offset until the natural-language task workflow is enabled.

```text
todo submit DBMS assignment | due 2026-09-18T17:00:00+05:30 | remind 2026-09-18T16:00:00+05:30 | note upload the PDF
tasks
done 1
remind 2026-09-18T16:00:00+05:30 | call Abhinav
```

The connected daemon checks due reminders every `REMINDER_POLL_SECONDS` seconds, sends the message to the configured self-chat, and records delivery in SQLite. A failed send remains recoverable instead of being silently discarded. When `N8N_KEEP_WEBHOOK_PATH` is configured, each new task is also written to Google Keep through n8n. Keep sync is durable and retried independently from task creation.

## Calendar details and recurrence

Agenda text, notes, and other supplied event details become the Google Calendar description. Explicit recurring requests are supported through validated RFC 5545 rules in n8n. For example, `cal add a recurring event every Wednesday at 3pm to solve the NPTEL assignment` becomes a weekly Wednesday event. Vague repetition or time language is rejected rather than silently creating an incorrect series.

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

Use a private network or localhost for n8n whenever possible. Protect its management UI independently.

## Full-pipeline checklist

- [ ] n8n calendar and confirmation webhooks authenticate the bearer token.
- [ ] n8n records `X-Idempotency-Key` before Calendar creation.
- [ ] `cargo run -- test-calendar "cal test event tomorrow at 6pm"` returns a created result.
- [ ] A deliberately ambiguous request returns `needs_confirmation`.
- [ ] A confirmation request creates exactly one Calendar event.
- [ ] Set `OWNER_JID` to the exact canonical JID shown by WhatsApp pairing diagnostics.
- [ ] Run `cargo run -- run`, scan the QR code, and verify a session database is created with restrictive permissions.
- [ ] Send `cal test event tomorrow at 6pm` from the linked account's self-chat and receive one concise reply.
- [ ] Publish the `WhatsApp Calendar - Voice` n8n workflow.
- [ ] Send a real voice note and verify one Calendar event and one WhatsApp confirmation.
- [ ] Send a clear recurring request and verify the event shows as repeating in Google Calendar.
- [ ] Add a task, list it, complete it, and verify one scheduled reminder reaches the self-chat.
