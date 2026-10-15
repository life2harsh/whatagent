# Architecture

The daemon connects WhatsApp to Google:

```text
WhatsApp adapter -> IncomingCommand -> command router -> CalendarService (Gemini -> validate -> Calendar API, or a task) -> reply -> WhatsApp adapter
```

`IncomingCommand` and `CommandContent` are application-owned types. The `whatsapp` module normalizes library events immediately, keeping the rest of the program independent of `whatsapp-rust` types.

`google::gemini` sends text or inline audio to Gemini with a JSON response schema and gets back one structured event reading. `calendar::validate` applies deterministic rules (title present, RFC 3339 times with offsets, end after start, not more than a day in the past, a constrained RFC 5545 subset for recurrence) and decides between creating, asking for confirmation, or declining. `google::calendar` inserts the event, authenticated by `google::auth`, which signs service-account JWTs with `ring` and caches the resulting access tokens. Google Calendar is optional: without it, an event becomes a task due at its start, with a reminder 30 minutes before (only the first occurrence of a repeating event).

SQLite is intentionally limited to task and delivery state:

- `processed_messages` claims WhatsApp message IDs and tracks delivery state.
- `pending_confirmations` survives daemon restarts until expiry.
- `tasks` stores open/completed user tasks, notes, due times, and reminder times.
- `reminder_outbox` provides durable claim/send/ack delivery for WhatsApp reminders.
- `google_keep_outbox` makes task-to-Keep synchronization durable without treating SQLite as a notes archive.

All clients share one `reqwest::Client` with a configured timeout. Calendar event IDs are the hex-encoded WhatsApp message ID, so Google itself rejects a duplicate insert (HTTP 409), which the agent reports as "already added". Confirmed events reuse the original message's ID for the same reason.

The adapter uses the `whatsapp-rust` 0.7 `Bot` callback API with its SQLite session store, QR pairing callback, reconnecting run loop, and same-chat reply method. Raw `MessageContext` exists only in `whatsapp`; it is normalized before the application router runs. The adapter keeps outbound message IDs in memory to prevent confirmation replies from re-entering the command path. Voice notes are downloaded and decrypted by the adapter and sent to Gemini inline. No chat monitoring or general chat behavior belongs in the daemon.

The reminder dispatcher starts only after WhatsApp connects. It claims due messages from SQLite, sends them through the connected client, then records delivery. A daemon crash or temporary disconnect leaves unacknowledged messages recoverable after a short claim timeout.

Google Keep sits behind `keep-bridge/`, a small localhost Python service using the unofficial gkeepapi library (the official Keep API is Workspace-only). Rust sends typed title/body payloads with a bearer token and a per-task idempotency key. A successful response stores the returned note ID; failed or unavailable calls remain pending for retry.
