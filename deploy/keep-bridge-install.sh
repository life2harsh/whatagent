#!/usr/bin/env bash
set -euo pipefail
src=${1:?source directory}
mode=${2:-full}
APP=/opt/wa-calendar-agent
DIR=$APP/keep-bridge
ENV=/etc/wa-calendar-keep-bridge.env
AGENT_ENV=/etc/wa-calendar-agent.env
RUN_USER=wa-calendar
umask 077

if ! id -u "$RUN_USER" > /dev/null 2>&1 || [[ ! -f $AGENT_ENV ]]; then
    echo "wa-calendar-agent isn't installed on this server; run deploy/install.sh first" >&2
    exit 1
fi
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
    if systemctl is-active --quiet wa-calendar-agent; then
        systemctl restart wa-calendar-agent
    fi
fi
