import base64
import getpass
import html
import json
import logging
import secrets
import sys
import time

import gpsoauth

SETUP_URL = "https://accounts.google.com/EmbeddedSetup"
SIGN_IN_TIMEOUT_SECONDS = 600
CODE_PREFIX = "keep1:"

PAGE_STYLE = """
<style>
  :root { color-scheme: light dark; }
  body { font: 16px/1.5 system-ui, sans-serif; max-width: 34em; margin: 3em auto; padding: 0 1.5em; }
  input, textarea, button { font: inherit; padding: .6em .8em; border-radius: 8px; border: 1px solid #8888; }
  input { width: 100%; box-sizing: border-box; margin: .5em 0 1em; }
  textarea { width: 100%; box-sizing: border-box; height: 7em; font-family: monospace; font-size: 13px; }
  button { background: #1a73e8; color: white; border: 0; cursor: pointer; margin-top: .5em; }
  .note { opacity: .75; font-size: 14px; }
</style>
"""

def email_page():
    return (
        "<!doctype html><meta charset=utf-8>"
        + PAGE_STYLE
        + f"""
<h1>Connect Google Keep</h1>
<p>Which Google account should your todos go to?</p>
<form id=form>
  <input id=email type=email required placeholder="you@gmail.com" autofocus>
  <button>Continue to Google sign-in</button>
</form>
<p class=note>Next, sign in to that account and click "I agree". The page may
keep loading after that; that's fine, this window moves on by itself.</p>
<script>
  document.getElementById("form").addEventListener("submit", async (event) => {{
    event.preventDefault();
    await window.pywebview.api.begin(document.getElementById("email").value.trim());
    window.location.href = {json.dumps(SETUP_URL)};
  }});
</script>
"""
    )


def code_page(code):
    return (
        "<!doctype html><meta charset=utf-8>"
        + PAGE_STYLE
        + f"""
<h1>Your Keep code</h1>
<p>Copy this and paste it into the installer on your server when it asks for
the Google Keep code.</p>
<textarea id=code readonly>{html.escape(code)}</textarea>
<button onclick="const c = document.getElementById('code'); c.select();
  document.execCommand('copy'); this.textContent = 'Copied'">Copy</button>
<p class=note>Treat it like a password: it gives access to your Google account.
Close this window once you've pasted it.</p>
"""
    )


def message_page(title, text):
    return (
        "<!doctype html><meta charset=utf-8>"
        + PAGE_STYLE
        + f"<h1>{html.escape(title)}</h1><p>{html.escape(text)}</p>"
    )


def say(message):
    if sys.stderr:
        print(message, file=sys.stderr, flush=True)


def ask(prompt, secret=False):
    if sys.platform == "win32":
        return (getpass.getpass(prompt) if secret else input(prompt)).strip()
    with open("/dev/tty") as tty_in, open("/dev/tty", "w") as tty_out:
        if secret:
            return getpass.getpass(prompt, stream=tty_out).strip()
        tty_out.write(prompt)
        tty_out.flush()
        return tty_in.readline().strip()


def exchange(email, cookie):
    if not cookie.startswith("oauth2_4/"):
        return None, "That doesn't look like the oauth_token value (it should start with oauth2_4/)."
    try:
        response = gpsoauth.exchange_token(email, cookie, secrets.token_hex(8))
    except Exception as error:
        return None, f"Couldn't reach Google ({error})."
    if "Token" not in response:
        return None, (
            f"Google refused the sign-in ({response.get('Error', 'unknown error')}). "
            "Check the email address, then try again; the sign-in only stays valid for a few minutes."
        )
    return response["Token"], None


def make_code(email, master_token):
    raw = f"{email}\n{master_token}".encode()
    return CODE_PREFIX + base64.urlsafe_b64encode(raw).decode().rstrip("=")


def sign_in_with_window(show_code):
    try:
        import webview
    except ImportError:
        return None
    logging.getLogger("pywebview").setLevel(logging.CRITICAL)
    state = {}

    class Api:
        def begin(self, email):
            state["email"] = email

    def watch(window):
        deadline = time.monotonic() + SIGN_IN_TIMEOUT_SECONDS
        while time.monotonic() < deadline and not state.get("closed"):
            cookie = None
            if "email" in state:
                try:
                    for jar in window.get_cookies() or []:
                        morsel = jar.get("oauth_token")
                        if morsel and morsel.value.startswith("oauth2_4/"):
                            cookie = morsel.value
                except Exception:
                    pass
            if cookie:
                token, error = exchange(state["email"], cookie)
                if error:
                    window.load_html(message_page("That didn't work", error + " Close this window and run it again."))
                    return
                state["token"] = token
                if show_code:
                    window.load_html(code_page(make_code(state["email"], token)))
                else:
                    window.destroy()
                return
            time.sleep(1)
        if not state.get("closed"):
            window.destroy()

    try:
        window = webview.create_window(
            "Connect Google Keep", html=email_page(), js_api=Api(), width=980, height=760
        )
        window.events.closed += lambda: state.__setitem__("closed", True)
        say("A window opened: enter your Gmail address, sign in, then click \"I agree\".")
        webview.start(watch, window, private_mode=True)
    except Exception as error:
        say(f"Couldn't open a sign-in window ({error}).")
        return None
    if "token" not in state:
        sys.exit("The sign-in didn't finish. Run it again when you're ready.")
    return state["email"], state["token"]


def sign_in_in_terminal():
    email = ask("Google account email: ")
    say("")
    say(f"  1. Open a private/incognito window and go to {SETUP_URL}")
    say('  2. Sign in, then click "I agree". The page may keep loading; that\'s fine.')
    say("  3. Press F12, then go to Application > Cookies > https://accounts.google.com")
    say('  4. Copy the value of the cookie named "oauth_token" (it starts with oauth2_4/).')
    say("     It only works for a few minutes, so paste it right away.")
    say("")
    token, error = exchange(email, ask("oauth_token value (hidden as you paste): ", secret=True))
    if error:
        sys.exit(error + " Run it again.")
    return email, token


def main():
    args = sys.argv[1:]
    env_output = "--env" in args
    result = None if "--manual" in args else sign_in_with_window(show_code=not env_output)
    if result is None:
        say("Falling back to copying the cookie by hand.")
        result = sign_in_in_terminal()
    email, token = result
    if env_output:
        print(f"KEEP_EMAIL={email}")
        print(f"KEEP_MASTER_TOKEN={token}")
    elif sys.stdout:
        print(f"Your Keep code (paste it into the installer):\n{make_code(email, token)}")


if __name__ == "__main__":
    if getattr(sys, "frozen", False):
        try:
            main()
        except SystemExit as stop:
            if stop.code not in (None, 0):
                say(str(stop.code))
        input("\nPress Enter to close this window.")
    else:
        main()
