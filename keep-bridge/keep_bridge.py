"""Localhost bridge that creates Google Keep notes for wa-calendar-agent.

Google's official Keep API only works for Workspace accounts, so this uses the
unofficial gkeepapi library with a master token. It speaks the same contract as
the Rust GoogleKeepWebhookClient:

    POST /notes   Authorization: Bearer <KEEP_BRIDGE_TOKEN>
                  X-Idempotency-Key: keep-task-<id>
                  {"task_id": 7, "chat_id": "...", "title": "...", "body": "..."}
    200 -> {"status": "created", "note": {"name": "<keep id>", "title": "..."}, "message": "..."}

Failures return a non-2xx status; the Rust outbox retries them later.
"""

import hmac
import json
import logging
import os
import sqlite3
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer

import gkeepapi

log = logging.getLogger("keep-bridge")

EMAIL = os.environ["KEEP_EMAIL"]
MASTER_TOKEN = os.environ["KEEP_MASTER_TOKEN"]
BRIDGE_TOKEN = os.environ["KEEP_BRIDGE_TOKEN"]
STATE_PATH = os.environ.get("KEEP_STATE_PATH", "keep-state.json")
DATABASE_PATH = os.environ.get("KEEP_BRIDGE_DATABASE_PATH", "keep-bridge.db")
PORT = int(os.environ.get("KEEP_BRIDGE_PORT", "8765"))
MAX_BODY_BYTES = 64 * 1024


class KeepSession:
    """Owns the gkeepapi client. Requests are served one at a time, so no locking."""

    def __init__(self):
        self.keep = None
        self.db = sqlite3.connect(DATABASE_PATH)
        self.db.execute(
            "CREATE TABLE IF NOT EXISTS notes ("
            " idempotency_key TEXT PRIMARY KEY NOT NULL,"
            " note_id TEXT NOT NULL,"
            " title TEXT NOT NULL)"
        )
        self.db.commit()

    def _login(self):
        keep = gkeepapi.Keep()
        state = None
        if os.path.exists(STATE_PATH):
            try:
                with open(STATE_PATH) as handle:
                    state = json.load(handle)
            except (OSError, ValueError):
                log.warning("ignoring unreadable Keep state cache")
        keep.authenticate(EMAIL, MASTER_TOKEN, state=state)
        self.keep = keep
        log.info("authenticated to Google Keep as %s", EMAIL)

    def _save_state(self):
        temporary = STATE_PATH + ".partial"
        with open(temporary, "w") as handle:
            json.dump(self.keep.dump(), handle)
        os.chmod(temporary, 0o600)
        os.replace(temporary, STATE_PATH)

    def create_note(self, idempotency_key, title, body):
        existing = self.db.execute(
            "SELECT note_id, title FROM notes WHERE idempotency_key = ?",
            (idempotency_key,),
        ).fetchone()
        if existing:
            return existing[0], existing[1]

        for attempt in (1, 2):
            try:
                if self.keep is None:
                    self._login()
                note = self.keep.createNote(title, body)
                self.keep.sync()
                break
            except Exception:
                # A stale session is the usual cause; log in again once.
                self.keep = None
                if attempt == 2:
                    raise
                log.warning("Keep sync failed; re-authenticating", exc_info=True)

        self.db.execute(
            "INSERT INTO notes (idempotency_key, note_id, title) VALUES (?, ?, ?)",
            (idempotency_key, note.id, title),
        )
        self.db.commit()
        try:
            self._save_state()
        except OSError:
            log.warning("could not save Keep state cache", exc_info=True)
        return note.id, title


SESSION = None


class Handler(BaseHTTPRequestHandler):
    def do_POST(self):
        if self.path != "/notes":
            return self._reply(404, {"status": "error", "message": "not found"})
        expected = f"Bearer {BRIDGE_TOKEN}"
        if not hmac.compare_digest(self.headers.get("Authorization", ""), expected):
            return self._reply(401, {"status": "error", "message": "unauthorized"})
        length = int(self.headers.get("Content-Length") or 0)
        if length <= 0 or length > MAX_BODY_BYTES:
            return self._reply(400, {"status": "error", "message": "invalid body size"})
        try:
            request = json.loads(self.rfile.read(length))
            title = str(request["title"]).strip()
            body = str(request.get("body") or "")
            key = self.headers.get("X-Idempotency-Key") or f"keep-task-{int(request['task_id'])}"
        except (ValueError, KeyError, TypeError):
            return self._reply(400, {"status": "error", "message": "invalid request"})
        if not title:
            return self._reply(400, {"status": "error", "message": "title is required"})

        try:
            note_id, note_title = SESSION.create_note(key, title, body)
        except Exception as error:
            log.exception("Keep note creation failed")
            return self._reply(502, {"status": "error", "message": f"Keep failed: {error}"})
        log.info("created Keep note %s for %s", note_id, key)
        self._reply(
            200,
            {
                "status": "created",
                "note": {"name": note_id, "title": note_title},
                "message": "Added to Google Keep",
            },
        )

    def _reply(self, status, payload):
        data = json.dumps(payload).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        self.end_headers()
        self.wfile.write(data)

    def log_message(self, format, *args):
        log.debug(format, *args)


def main():
    global SESSION
    logging.basicConfig(
        level=os.environ.get("KEEP_BRIDGE_LOG_LEVEL", "INFO"),
        format="%(levelname)s %(name)s: %(message)s",
        stream=sys.stdout,
    )
    SESSION = KeepSession()
    server = HTTPServer(("127.0.0.1", PORT), Handler)
    log.info("listening on 127.0.0.1:%d", PORT)
    server.serve_forever()


if __name__ == "__main__":
    main()
