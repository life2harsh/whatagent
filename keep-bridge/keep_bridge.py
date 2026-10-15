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
LIST_TITLE = os.environ.get("KEEP_LIST_TITLE", "WhatsApp Todos")
PORT = int(os.environ.get("KEEP_BRIDGE_PORT", "8765"))
MAX_BODY_BYTES = 64 * 1024


class KeepSession:
    def __init__(self):
        self.keep = None
        self.db = sqlite3.connect(DATABASE_PATH)
        self.db.execute(
            "CREATE TABLE IF NOT EXISTS notes ("
            " idempotency_key TEXT PRIMARY KEY NOT NULL,"
            " note_id TEXT NOT NULL,"
            " title TEXT NOT NULL)"
        )
        self.db.execute(
            "CREATE TABLE IF NOT EXISTS lists ("
            " title TEXT PRIMARY KEY NOT NULL,"
            " list_id TEXT NOT NULL)"
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

    def check(self):
        self._login()
        self._save_state()
        return sum(1 for _ in self.keep.all())

    def _remember_list(self, list_id):
        self.db.execute(
            "INSERT INTO lists (title, list_id) VALUES (?, ?)"
            " ON CONFLICT(title) DO UPDATE SET list_id = excluded.list_id",
            (LIST_TITLE, list_id),
        )
        self.db.commit()

    def _find_list(self):
        row = self.db.execute(
            "SELECT list_id FROM lists WHERE title = ?", (LIST_TITLE,)
        ).fetchone()
        if row:
            node = self.keep.get(row[0])
            if node is not None and not node.trashed and not node.deleted:
                return node
        for node in self.keep.all():
            if (
                isinstance(node, gkeepapi.node.List)
                and node.title == LIST_TITLE
                and not node.trashed
                and not node.deleted
            ):
                self._remember_list(node.id)
                return node
        return None

    def add_list_item(self, idempotency_key, text):
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
                node = self._find_list()
                if node is None:
                    self.keep.sync()
                    node = self._find_list()
                if node is None:
                    node = self.keep.createList(LIST_TITLE, [(text, False)])
                    self._remember_list(node.id)
                    item = node.items[0]
                else:
                    item = node.add(text, False)
                self.keep.sync()
                break
            except Exception:
                self.keep = None
                if attempt == 2:
                    raise
                log.warning("Keep list sync failed; re-authenticating", exc_info=True)

        self.db.execute(
            "INSERT INTO notes (idempotency_key, note_id, title) VALUES (?, ?, ?)",
            (idempotency_key, item.id, text),
        )
        self.db.commit()
        try:
            self._save_state()
        except OSError:
            log.warning("could not save Keep state cache", exc_info=True)
        return item.id, text


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
            key = self.headers.get("X-Idempotency-Key") or f"keep-task-{int(request['task_id'])}"
        except (ValueError, KeyError, TypeError):
            return self._reply(400, {"status": "error", "message": "invalid request"})
        if not title:
            return self._reply(400, {"status": "error", "message": "title is required"})

        try:
            item_id, item_text = SESSION.add_list_item(key, title)
        except Exception as error:
            log.exception("Keep list update failed")
            return self._reply(502, {"status": "error", "message": f"Keep failed: {error}"})
        log.info("added Keep list item %s for %s", item_id, key)
        self._reply(
            200,
            {
                "status": "created",
                "note": {"name": item_id, "title": item_text},
                "message": f"Added to your Google Keep list \"{LIST_TITLE}\"",
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
    if "--check" in sys.argv[1:]:
        try:
            count = SESSION.check()
        except gkeepapi.exception.LoginException as error:
            sys.exit(
                f"Google didn't accept the Keep sign-in for {EMAIL} ({error}). "
                "Get a fresh Keep code and try again."
            )
        except Exception as error:
            sys.exit(f"Couldn't reach Google Keep ({error}).")
        print(f"Logged in to Google Keep as {EMAIL} ({count} notes)")
        return
    server = HTTPServer(("127.0.0.1", PORT), Handler)
    log.info("listening on 127.0.0.1:%d", PORT)
    server.serve_forever()


if __name__ == "__main__":
    main()
