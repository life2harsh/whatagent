#!/usr/bin/env bash
# Turns on Google Keep sync for wa-calendar-agent, from your machine.
#
#   deploy/keep-setup.sh user@host [-i ssh-key]
#   deploy/keep-setup.sh user@host [-i ssh-key] --update
#
# Default: walks you through a Google sign-in, gets a Keep master token,
# installs keep-bridge on the server, checks it can log in, and points the
# agent at it. The token travels over ssh stdin; it is never printed or
# written to disk on this machine. Re-run it to replace an expired token.
# With --update: redeploys the keep-bridge code only, reusing the token already
# on the server (no sign-in). Use it after changing the bridge.
set -euo pipefail

usage() { sed -n '2,12p' "$0" | sed 's/^# \{0,1\}//'; exit "${1:-0}"; }

here=$(cd "$(dirname "$0")" && pwd)
host=""
mode=full
ssh_opts=(-o ConnectTimeout=15)
while (($#)); do
    case $1 in
        -i | --ssh-key) ssh_opts+=(-i "${2:?$1 needs a file}"); shift 2 ;;
        --update) mode=update; shift ;;
        -h | --help) usage ;;
        -*) echo "unknown option: $1" >&2; usage 2 ;;
        *) [[ -z $host ]] || usage 2; host=$1; shift ;;
    esac
done
[[ -n $host ]] || usage 2

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
email=""
master_token=""

if [[ $mode == full ]]; then
    command -v python3 > /dev/null || { echo "python3 is required on this machine" >&2; exit 1; }
    echo "Preparing a temporary Python environment..."
    # The sign-in window uses the system's WebKitGTK bindings; without them the
    # helper falls back to copying the cookie by hand.
    webview_ok=""
    if [[ $(uname) == Linux ]] && { [[ -n ${DISPLAY:-} ]] || [[ -n ${WAYLAND_DISPLAY:-} ]]; }; then
        if ! python3 -c 'import gi; gi.require_version("WebKit2", "4.1")' 2> /dev/null; then
            if command -v apt-get > /dev/null; then pkgs=(apt-get install -y python3-gi gir1.2-webkit2-4.1)
            elif command -v pacman > /dev/null; then pkgs=(pacman -S --needed --noconfirm python-gobject webkit2gtk-4.1)
            elif command -v dnf > /dev/null; then pkgs=(dnf install -y python3-gobject webkit2gtk4.1)
            else pkgs=(); fi
            if ((${#pkgs[@]})); then
                echo "The sign-in window needs ${pkgs[*]:3}. Installing (may ask for your password)..."
                sudo "${pkgs[@]}" > /dev/null 2>&1 || true
            fi
        fi
        python3 -c 'import gi; gi.require_version("WebKit2", "4.1")' 2> /dev/null && webview_ok=1
    fi
    python3 -m venv --system-site-packages "$work/venv"
    "$work/venv/bin/pip" install -q --disable-pip-version-check gpsoauth
    [[ -n $webview_ok ]] && "$work/venv/bin/pip" install -q --disable-pip-version-check pywebview

    # Opens a sign-in window and captures the cookie (falls back to pasting it).
    login=$("$work/venv/bin/python" "$here/../keep-bridge/get_master_token.py")
    # Parameter expansion, not a here-string, so the token never touches a temp file.
    email=${login#*KEEP_EMAIL=}
    email=${email%%$'\n'*}
    master_token=${login#*KEEP_MASTER_TOKEN=}
    master_token=${master_token%%$'\n'*}
    unset login
    [[ -n $email && -n $master_token ]] || { echo "didn't get a master token" >&2; exit 1; }
    echo "Got a master token."
else
    echo "Updating the keep-bridge code on $host (reusing the token already there)..."
fi

cat > "$work/remote.sh" << 'REMOTE'
set -euo pipefail
src=$1
mode=${2:-full}
APP=/opt/wa-calendar-agent
DIR=$APP/keep-bridge
ENV=/etc/wa-calendar-keep-bridge.env
AGENT_ENV=/etc/wa-calendar-agent.env
RUN_USER=wa-calendar
umask 077

id -u "$RUN_USER" > /dev/null 2>&1 && [[ -f $AGENT_ENV ]] \
    || { echo "wa-calendar-agent isn't installed on this server; run deploy/install.sh first" >&2; exit 1; }
[[ $mode == update && ! -f $ENV ]] \
    && { echo "Keep isn't set up yet; run without --update first" >&2; exit 1; }
env_set() {
    KEY=$2 VALUE=$3 awk 'BEGIN { done = 0 }
        index($0, ENVIRON["KEY"] "=") == 1 { if (!done) print ENVIRON["KEY"] "=" ENVIRON["VALUE"]; done = 1; next }
        { print }
        END { if (!done) print ENVIRON["KEY"] "=" ENVIRON["VALUE"] }' "$1" > "$1.tmp"
    cat "$1.tmp" > "$1" && rm -f "$1.tmp"
}

echo "Installing keep-bridge..."
python3 -c 'import ensurepip' 2> /dev/null || apt-get install -y -qq python3-venv > /dev/null
install -d -m 0750 -o "$RUN_USER" -g "$RUN_USER" "$DIR"
install -m 0644 -o "$RUN_USER" -g "$RUN_USER" "$src/keep_bridge.py" "$src/requirements.txt" "$DIR/"
[[ -x $DIR/venv/bin/python ]] || runuser -u "$RUN_USER" -- python3 -m venv "$DIR/venv"
runuser -u "$RUN_USER" -- "$DIR/venv/bin/pip" install -q --no-cache-dir --disable-pip-version-check \
    --upgrade -r "$DIR/requirements.txt"

if [[ $mode == full ]]; then
    read -r email
    read -r master_token
    # Reuse the existing bridge token on a re-run; the env file is absent first time.
    bridge_token=""
    [[ -f $ENV ]] && bridge_token=$(sed -n 's/^KEEP_BRIDGE_TOKEN=//p' "$ENV" | tail -1)
    [[ -n $bridge_token ]] || bridge_token=$(python3 -c 'import secrets; print(secrets.token_hex(32))')
    cat > "$ENV.new" << EOF
KEEP_EMAIL=$email
KEEP_MASTER_TOKEN=$master_token
KEEP_BRIDGE_TOKEN=$bridge_token
KEEP_BRIDGE_PORT=8765
KEEP_STATE_PATH=$DIR/keep-state.json
KEEP_BRIDGE_DATABASE_PATH=$DIR/keep-bridge.db
EOF
    chown "$RUN_USER:$RUN_USER" "$ENV.new"
    chmod 0600 "$ENV.new"
    mv -f "$ENV.new" "$ENV"
    unset master_token
    # A fresh token invalidates the cached session.
    rm -f "$DIR/keep-state.json"
fi

echo "Checking the Google Keep login (the first sync can take a little while)..."
if ! systemd-run --quiet --wait --collect --pipe -p User="$RUN_USER" -p Group="$RUN_USER" \
    -p WorkingDirectory="$DIR" -p EnvironmentFile="$ENV" \
    "$DIR/venv/bin/python" keep_bridge.py --check 2>&1 | grep -v '^INFO'; then
    echo "Keep login failed; the agent was left unchanged." >&2
    exit 1
fi

install -m 0644 "$src/keep-bridge.service" /etc/systemd/system/keep-bridge.service
systemctl daemon-reload
systemctl enable keep-bridge > /dev/null 2>&1
systemctl restart keep-bridge
for _ in $(seq 20); do
    ss -ltn | grep -q '127.0.0.1:8765' && break
    sleep 1
done
ss -ltn | grep -q '127.0.0.1:8765' \
    || { journalctl -u keep-bridge -n 20 --no-pager; echo "keep-bridge didn't start" >&2; exit 1; }

if [[ $mode == full ]]; then
    cp -p "$AGENT_ENV" "$AGENT_ENV.bak-$(date -u +%Y%m%dT%H%M%SZ)"
    env_set "$AGENT_ENV" KEEP_BRIDGE_URL http://127.0.0.1:8765/notes
    env_set "$AGENT_ENV" KEEP_BRIDGE_TOKEN "$bridge_token"
    systemctl restart wa-calendar-agent
fi
rm -rf "$src"
echo
if [[ $mode == full ]]; then
    echo "Done. Send \"todo test keep sync\" to yourself on WhatsApp; it should show up in Google Keep."
else
    echo "Done. The keep-bridge code is updated and restarted."
fi
REMOTE

remote=$(ssh "${ssh_opts[@]}" "$host" 'mktemp -d')
echo "Uploading keep-bridge to $host..."
scp -q "${ssh_opts[@]}" "$here/../keep-bridge/keep_bridge.py" "$here/../keep-bridge/requirements.txt" \
    "$here/keep-bridge.service" "$work/remote.sh" "$host:$remote/"
# $remote is the server's own temp dir, so expanding it here is intended.
# shellcheck disable=SC2029
if [[ $mode == full ]]; then
    printf '%s\n%s\n' "$email" "$master_token" \
        | ssh "${ssh_opts[@]}" "$host" "sudo bash $remote/remote.sh $remote full; rc=\$?; sudo rm -rf $remote; exit \$rc"
else
    ssh "${ssh_opts[@]}" "$host" "sudo bash $remote/remote.sh $remote update; rc=\$?; sudo rm -rf $remote; exit \$rc"
fi
