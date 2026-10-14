"""Gets a Google Keep master token for keep-bridge.

Opens a small sign-in window (an embedded web view via pywebview, the same
kind ytuff uses). You sign in to your own Google account and click "I agree";
Google then sets a one-time `oauth_token` cookie, which this reads straight
from the web view and exchanges for a master token with gpsoauth (the flow
gkeepapi documents). The web view session lives in memory only. Without a
display, or with --manual, it asks you to paste the cookie instead.

Prints KEEP_EMAIL=... and KEEP_MASTER_TOKEN=... on stdout; everything else
goes to stderr. The master token grants broad access to the Google account:
store it only in /etc/wa-calendar-keep-bridge.env (mode 0600).
"""

import getpass
import logging
import secrets
import sys
import time

import gpsoauth

SETUP_URL = "https://accounts.google.com/EmbeddedSetup"
SIGN_IN_TIMEOUT_SECONDS = 600


def say(message):
    print(message, file=sys.stderr, flush=True)


def ask(prompt, secret=False):
    # Separate read and write handles: stdout is captured by keep-setup.sh,
    # and /dev/tty can't be opened "r+" because it isn't seekable.
    with open("/dev/tty") as tty_in, open("/dev/tty", "w") as tty_out:
        if secret:
            return getpass.getpass(prompt, stream=tty_out).strip()
        tty_out.write(prompt)
        tty_out.flush()
        return tty_in.readline().strip()


def capture_cookie_in_window():
    """Opens the sign-in window and returns the oauth_token cookie, or None if
    no web view is available here. Reads the cookie from the web view's own
    cookie store; it never injects anything into Google's page."""
    try:
        import webview
    except ImportError:
        return None
    # pywebview logs a harmless error when its bootstrap script meets Google's
    # Trusted Types policy; the cookie read below doesn't depend on it.
    logging.getLogger("pywebview").setLevel(logging.CRITICAL)
    found = {}

    def watch(window):
        deadline = time.monotonic() + SIGN_IN_TIMEOUT_SECONDS
        while time.monotonic() < deadline and not found.get("closed"):
            try:
                for jar in window.get_cookies() or []:
                    morsel = jar.get("oauth_token")
                    if morsel and morsel.value.startswith("oauth2_4/"):
                        found["cookie"] = morsel.value
                        break
            except Exception:
                pass
            if "cookie" in found:
                break
            time.sleep(1)
        window.destroy()

    try:
        window = webview.create_window(
            'Sign in to Google, then click "I agree"', SETUP_URL, width=980, height=760
        )
        window.events.closed += lambda: found.__setitem__("closed", True)
        say('A sign-in window opened. Sign in to the account whose Keep you want, then click "I agree".')
        say("The page may keep loading after that; that's fine, the window closes on its own.")
        # private_mode keeps the session in memory only, so nothing is left on disk.
        webview.start(watch, window, private_mode=True)
    except Exception as error:
        say(f"Couldn't open a sign-in window ({error}).")
        return None
    return found.get("cookie")


def get_cookie_manually():
    say("")
    say(f"  1. Open a private/incognito window and go to {SETUP_URL}")
    say('  2. Sign in, then click "I agree". The page may keep loading; that\'s fine.')
    say("  3. Press F12, then go to Application > Cookies > https://accounts.google.com")
    say('  4. Copy the value of the cookie named "oauth_token" (it starts with oauth2_4/).')
    say("     It only works for a few minutes, so paste it right away.")
    say("")
    return ask("oauth_token value (hidden as you paste): ", secret=True)


def main():
    manual = "--manual" in sys.argv[1:]
    email = ask("Google account email: ")
    cookie = None if manual else capture_cookie_in_window()
    if not cookie:
        if not manual:
            say("Falling back to copying the cookie by hand.")
        cookie = get_cookie_manually()
    if not cookie.startswith("oauth2_4/"):
        sys.exit("That doesn't look like the oauth_token value (it should start with oauth2_4/). Run it again.")
    response = gpsoauth.exchange_token(email, cookie, secrets.token_hex(8))
    if "Token" not in response:
        sys.exit(
            f"Google refused the sign-in ({response.get('Error', 'unknown error')}). "
            "The cookie expires quickly; sign in again and try once more."
        )
    print(f"KEEP_EMAIL={email}")
    print(f"KEEP_MASTER_TOKEN={response['Token']}")


if __name__ == "__main__":
    main()
