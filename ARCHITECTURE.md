# Architecture

The daemon is a deterministic boundary between WhatsApp and n8n:

```text
WhatsApp adapter -> IncomingCommand -> command router -> n8n client -> response formatter -> WhatsApp adapter
```

`IncomingCommand` and `CommandContent` are application-owned types. The future `whatsapp` module will normalize library events immediately, keeping the rest of the program independent of `whatsapp-rust` types.

n8n owns speech-to-text, LLM extraction, validation, idempotent Calendar creation, and Google credentials. Rust owns WhatsApp authentication, owner/self-chat filtering, media transport, durable confirmation state, deduplication, and concise acknowledgements.

SQLite is intentionally limited to task and delivery state:

- `processed_messages` claims WhatsApp message IDs and tracks delivery state.
- `pending_confirmations` survives daemon restarts until expiry.
- `tasks` stores open/completed user tasks, notes, due times, and reminder times.
- `reminder_outbox` provides durable claim/send/ack delivery for WhatsApp reminders.
- `google_keep_outbox` makes task-to-Keep synchronization durable without treating SQLite as a notes archive.

The Rust client uses one reusable `reqwest::Client`, a configured timeout, typed serde contracts, bearer authentication, and `X-Idempotency-Key`. It does not retry create requests automatically: a timeout could occur after Google Calendar has already created the event.

Stage 2 uses the verified `whatsapp-rust` 0.7 `Bot` callback API with its SQLite session store, QR pairing callback, reconnecting run loop, and same-chat reply method. Raw `MessageContext` exists only in `whatsapp`; it is normalized before the application router runs. The adapter keeps outbound message IDs in memory to prevent confirmation replies from re-entering the command path. The audio request model is reserved for Stage 4, when the adapter will download and decrypt voice-note media. No Google OAuth, LLM provider coupling, local transcription, chat monitoring, or general chat behavior belongs in the daemon.

The reminder dispatcher starts only after WhatsApp connects. It claims due messages from SQLite, sends them through the connected client, then records delivery. A daemon crash or temporary disconnect leaves unacknowledged messages recoverable after a short claim timeout.

Google Keep remains behind n8n. Rust sends only typed title/body payloads to an authenticated webhook. n8n owns Google OAuth and calls the Keep API. A successful Keep response stores the returned note name; failed or unavailable calls remain pending for retry.
