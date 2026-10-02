"""One-time helper: exchange an EmbeddedSetup oauth_token for a Keep master token.

1. Open https://accounts.google.com/EmbeddedSetup in a browser and sign in.
2. After "I agree", read the `oauth_token` cookie (DevTools > Application > Cookies).
3. Run this script on a trusted machine and paste the values when asked.

The master token grants broad access to the Google account. Store it only in
/etc/wa-calendar-keep-bridge.env (mode 0600) and never commit it.
"""

import getpass
import secrets

import gpsoauth

email = input("Google account email: ").strip()
oauth_token = getpass.getpass("oauth_token cookie (starts with oauth2_4/): ").strip()
android_id = secrets.token_hex(8)
response = gpsoauth.exchange_token(email, oauth_token, android_id)
if "Token" not in response:
    raise SystemExit(f"Exchange failed: {response.get('Error', response)}")
print("\nKEEP_MASTER_TOKEN=" + response["Token"])
