#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat << 'EOF'
Installs or upgrades wa-calendar-agent on a Linux server with systemd.

  curl -fsSL https://raw.githubusercontent.com/life2harsh/whatagent/main/deploy/install.sh | sudo bash

The first run asks for your Gemini key and timezone, optionally connects
Google Calendar, shows a WhatsApp QR code, and starts the service. Re-running
it upgrades the binary and keeps your settings and WhatsApp session.

Options:
  --calendar              connect (or change) Google Calendar
  --keep                  connect (or reconnect) Google Keep
  --service-account FILE  connect Google Calendar with this service account key
                          (or a calendar code from deploy/google-setup.sh)
  --binary FILE           install this binary instead of downloading a release
  --version TAG           release to download (default: latest)
  --reconfigure           ask for every setting again
  --relink                link WhatsApp again (new QR code)
EOF
    exit 0
}

REPO=${WA_REPO:-life2harsh/whatagent}
APP_DIR=/opt/wa-calendar-agent
ENV_FILE=/etc/wa-calendar-agent.env
SERVICE=wa-calendar-agent
RUN_USER=wa-calendar
UNIT_FILE=/etc/systemd/system/$SERVICE.service

service_account=""
binary=""
version=latest
reconfigure=false
relink=false
calendar=false
keep=false
while (($#)); do
    case $1 in
        --service-account) service_account=${2:?--service-account needs a file}; shift 2 ;;
        --binary) binary=${2:?--binary needs a file}; shift 2 ;;
        --version) version=${2:?--version needs a tag}; shift 2 ;;
        --reconfigure) reconfigure=true; shift ;;
        --relink) relink=true; shift ;;
        --calendar) calendar=true; shift ;;
        --keep) keep=true; shift ;;
        -h | --help) usage ;;
        *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
done

step() { printf '\n\033[1m==> %s\033[0m\n' "$*"; }
die() { printf '\033[31merror:\033[0m %s\n' "$*" >&2; exit 1; }

[[ $EUID -eq 0 ]] || die "run this as root (sudo bash install.sh)"
command -v systemctl > /dev/null || die "systemd is required"
command -v curl > /dev/null || die "curl is required (apt install curl)"
umask 077

tty_ok() { [[ -r /dev/tty && -w /dev/tty ]] && { : < /dev/tty; } 2> /dev/null; }
ask() {
    local prompt=$1 default=${2:-} reply
    tty_ok || die "missing setting and no terminal to ask: $prompt"
    [[ -n $default ]] && prompt="$prompt [$default]"
    read -rp "$prompt: " reply < /dev/tty
    printf '%s' "${reply:-$default}"
}
ask_secret() {
    local reply
    tty_ok || die "missing setting and no terminal to ask: $1"
    read -rsp "$1: " reply < /dev/tty
    echo > /dev/tty
    printf '%s' "$reply"
}

env_get() {
    if [[ -f $ENV_FILE ]]; then
        sed -n "s/^$1=//p" "$ENV_FILE" | tail -1
    fi
}
env_set() {
    local key=$1 value=$2
    touch "$ENV_FILE"
    KEY=$key VALUE=$value awk 'BEGIN { done = 0 }
        index($0, ENVIRON["KEY"] "=") == 1 { if (!done) print ENVIRON["KEY"] "=" ENVIRON["VALUE"]; done = 1; next }
        { print }
        END { if (!done) print ENVIRON["KEY"] "=" ENVIRON["VALUE"] }' "$ENV_FILE" > "$ENV_FILE.tmp"
    mv "$ENV_FILE.tmp" "$ENV_FILE"
}

absolute() { [[ $1 == /* ]] && printf '%s' "$1" || printf '%s' "$APP_DIR/$1"; }

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

script_dir=""
[[ -f ${BASH_SOURCE[0]:-} ]] && script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
[[ -z $binary && -n $script_dir && -x $script_dir/wa-calendar-agent ]] && binary=$script_dir/wa-calendar-agent
bundle_dir=$script_dir

unit=""
[[ -n $script_dir && -f $script_dir/$SERVICE.service ]] && unit=$script_dir/$SERVICE.service

if [[ -z $binary ]]; then
    case $(uname -m) in
        x86_64 | amd64) arch=x86_64 ;;
        aarch64 | arm64) arch=aarch64 ;;
        *) die "no prebuilt binary for $(uname -m); build from source and pass --binary" ;;
    esac
    asset=wa-calendar-agent-$arch-linux.tar.gz
    if [[ $version == latest ]]; then
        base=https://github.com/$REPO/releases/latest/download
    else
        base=https://github.com/$REPO/releases/download/$version
    fi
    step "Downloading $asset ($version)"
    curl -fL --retry 3 --progress-bar -o "$work/$asset" "$base/$asset" \
        || die "download failed. Is there a published release at github.com/$REPO/releases?"
    curl -fsSL --retry 3 -o "$work/$asset.sha256" "$base/$asset.sha256"
    (cd "$work" && sha256sum -c --quiet "$asset.sha256") || die "checksum mismatch for $asset"
    tar -xzf "$work/$asset" -C "$work"
    binary=$work/wa-calendar-agent-$arch-linux/wa-calendar-agent
    unit=$work/wa-calendar-agent-$arch-linux/$SERVICE.service
    bundle_dir=$work/wa-calendar-agent-$arch-linux
fi
[[ -f $binary ]] || die "binary not found: $binary"
if [[ -z $unit ]]; then
    curl -fsSL --retry 3 -o "$work/$SERVICE.service" \
        "https://raw.githubusercontent.com/$REPO/main/deploy/$SERVICE.service" \
        || die "couldn't download the systemd unit"
    unit=$work/$SERVICE.service
fi
chmod 0755 "$binary"
"$binary" --help > /dev/null 2>&1 \
    || die "this binary can't run here (prebuilt releases need glibc 2.35+, e.g. Ubuntu 22.04+ or Debian 12+)"

step "Installing to $APP_DIR"
id -u "$RUN_USER" > /dev/null 2>&1 \
    || useradd --system --home "$APP_DIR" --shell /usr/sbin/nologin "$RUN_USER"
install -d -m 0750 -o "$RUN_USER" -g "$RUN_USER" "$APP_DIR"
if [[ -f $APP_DIR/wa-calendar-agent ]]; then
    backup=$APP_DIR/wa-calendar-agent.old-$(date -u +%Y%m%dT%H%M%SZ)
    cp -p "$APP_DIR/wa-calendar-agent" "$backup"
    backups=("$APP_DIR"/wa-calendar-agent.old-*)
    if ((${#backups[@]} > 3)); then rm -f "${backups[@]:0:${#backups[@]}-3}"; fi
fi
install -m 0755 "$binary" "$APP_DIR/wa-calendar-agent.new"
mv -f "$APP_DIR/wa-calendar-agent.new" "$APP_DIR/wa-calendar-agent"
install -m 0644 "$unit" "$UNIT_FILE"
systemctl daemon-reload

[[ -f $ENV_FILE ]] || install -m 0600 -o "$RUN_USER" -g "$RUN_USER" /dev/null "$ENV_FILE"
[[ -n $(env_get RUST_LOG) ]] || env_set RUST_LOG info
[[ -n $(env_get DATABASE_URL) ]] || env_set DATABASE_URL "sqlite://$APP_DIR/wa-calendar.db"
[[ -n $(env_get WHATSAPP_SESSION_DATABASE_PATH) ]] \
    || env_set WHATSAPP_SESSION_DATABASE_PATH "$APP_DIR/wa-session.db"

if [[ $reconfigure == true || -z $(env_get GEMINI_API_KEY) ]]; then
    step "Gemini"
    echo "Get a free API key at https://aistudio.google.com/apikey (or from deploy/google-setup.sh)"
    while :; do
        gemini_key=$(ask_secret "Gemini API key (hidden)")
        [[ -n $gemini_key ]] || continue
        if printf 'x-goog-api-key: %s\n' "$gemini_key" | curl -fsS -o /dev/null -H @- \
            "https://generativelanguage.googleapis.com/v1beta/models?pageSize=1" 2> /dev/null; then
            break
        fi
        echo "Gemini rejected that key. Try again."
    done
    env_set GEMINI_API_KEY "$gemini_key"
    unset gemini_key
fi

if [[ $reconfigure == true || -z $(env_get USER_TIMEZONE) ]]; then
    server_tz=$(timedatectl show -p Timezone --value 2> /dev/null || true)
    [[ $server_tz == UTC || $server_tz == Etc/UTC ]] && server_tz=""
    while :; do
        tz=$(ask "Your timezone, e.g. Asia/Kolkata or Europe/London" "${server_tz:-$(env_get USER_TIMEZONE)}")
        [[ -n $tz && ( ! -d /usr/share/zoneinfo || -f /usr/share/zoneinfo/$tz ) ]] && break
        echo "Unknown timezone: $tz"
    done
    env_set USER_TIMEZONE "$tz"
fi

calendar_off() {
    env_set GOOGLE_SERVICE_ACCOUNT_KEY_PATH ""
    env_set GOOGLE_CALENDAR_ID ""
}
want_calendar=false
if [[ -n $service_account || $calendar == true ]]; then
    want_calendar=true
elif [[ $reconfigure == true ]] || ! grep -q '^GOOGLE_CALENDAR_ID=' "$ENV_FILE"; then
    step "Google Calendar (optional)"
    cat << 'EOF'
Events can go straight into your Google Calendar. That needs a calendar code:
open https://shell.cloud.google.com in your browser and run
  curl -fsSL https://raw.githubusercontent.com/life2harsh/whatagent/main/deploy/google-setup.sh | bash
Without it, events become todos with a WhatsApp reminder 30 minutes before.
You can add it any time later by re-running this with --calendar.
EOF
    if tty_ok && [[ $(ask "Connect Google Calendar now? (y/N)") == [yY]* ]]; then
        want_calendar=true
    else
        calendar_off
    fi
fi

key_path=$(env_get GOOGLE_SERVICE_ACCOUNT_KEY_PATH)
key_path=$(absolute "${key_path:-$APP_DIR/service-account.json}")
use_calendar_key() {
    local input=$1 data
    if [[ $input == cal1:* ]]; then
        data=${input#cal1:}
        [[ $data =~ ^[A-Za-z0-9_-]+$ ]] || return 1
        while ((${#data} % 4)); do data+="="; done
        printf '%s' "$data" | tr '_-' '/+' | base64 -d 2> /dev/null | gunzip -c > "$work/code" 2> /dev/null \
            || return 1
        code_calendar_id=$(head -n1 "$work/code")
        tail -n +2 "$work/code" > "$work/key.json"
        rm -f "$work/code"
        input=$work/key.json
        [[ $code_calendar_id == *@* ]] || { code_calendar_id=""; return 1; }
    else
        input=${input/#\~/$HOME}
    fi
    [[ -f $input ]] && grep -q '"type": *"service_account"' "$input" || return 1
    key_path=$APP_DIR/service-account.json
    install -m 0600 -o "$RUN_USER" -g "$RUN_USER" "$input" "$key_path"
    rm -f "$work/key.json"
}
if [[ $want_calendar == true ]]; then
    step "Google Calendar"
    code_calendar_id=""
    if [[ -n $service_account ]]; then
        use_calendar_key "$service_account" || die "$service_account isn't a calendar code or a service account key file"
    elif [[ $calendar == true || $reconfigure == true || ! -f $key_path ]]; then
        cat << 'EOF'
Paste the calendar code from deploy/google-setup.sh (open https://shell.cloud.google.com
and run the command shown above), or type the path to a service account JSON key
you copied to this server.
EOF
        hint="empty to skip"
        [[ -f $key_path ]] && hint="empty to keep the current one"
        while :; do
            reply=$(ask_secret "Calendar code or key file (hidden, $hint)")
            [[ -z $reply ]] && break
            use_calendar_key "$reply" && break
            echo "That isn't a calendar code (it starts with cal1:) or a key file on this server. Try again."
        done
        unset reply
    fi
    if [[ -f $key_path ]]; then
        env_set GOOGLE_SERVICE_ACCOUNT_KEY_PATH "$key_path"
        if [[ -n $code_calendar_id ]]; then
            calendar_id=$code_calendar_id
            echo "Using the calendar $calendar_id."
        else
            calendar_id=$(ask "Calendar ID (your Gmail address for your main calendar)" "$(env_get GOOGLE_CALENDAR_ID)")
            [[ -n $calendar_id ]] || die "a calendar ID is required"
        fi
        env_set GOOGLE_CALENDAR_ID "$calendar_id"
        sa_email=$(sed -n 's/.*"client_email": *"\([^"]*\)".*/\1/p' "$key_path" | head -1)
        project=$(sed -n 's/.*"project_id": *"\([^"]*\)".*/\1/p' "$key_path" | head -1)
        cat << EOF

Share your calendar with whatagent (on a computer, not the phone app):
  1. Open https://calendar.google.com/calendar/r/settings
  2. On the left, under "Settings for my calendars", click $calendar_id
  3. "Share with specific people or groups" > "Add people and groups"
  4. Add ${sa_email:-the client_email from the key file}
     with "Make changes to events", then click Send
EOF
        [[ -n $code_calendar_id ]] \
            || echo "Also turn on the Calendar API: https://console.cloud.google.com/apis/library/calendar-json.googleapis.com?project=$project"
        tty_ok && ask "Press Enter once that's done" > /dev/null
    else
        echo "Skipping Google Calendar for now; events will become WhatsApp reminders."
        calendar_off
    fi
fi
keep_files() {
    local out=$work/keep-bridge file from candidate repo_dir candidates
    mkdir -p "$out"
    for file in keep_bridge.py requirements.txt keep-bridge.service keep-bridge-install.sh; do
        candidates=()
        [[ -n $bundle_dir ]] && candidates+=("$bundle_dir/keep-bridge/$file")
        [[ -n $script_dir ]] && candidates+=("$script_dir/../keep-bridge/$file" "$script_dir/$file")
        from=""
        for candidate in "${candidates[@]}"; do
            [[ -f $candidate ]] && { from=$candidate; break; }
        done
        if [[ -n $from ]]; then
            cp "$from" "$out/"
        else
            case $file in *.py | *.txt) repo_dir="keep-bridge" ;; *) repo_dir="deploy" ;; esac
            curl -fsSL --retry 3 -o "$out/$file" "https://raw.githubusercontent.com/$REPO/main/$repo_dir/$file" \
                || return 1
        fi
    done
    printf '%s' "$out"
}
decode_keep_code() {
    local data=${1#keep1:}
    [[ $1 == keep1:* && $data =~ ^[A-Za-z0-9_-]+$ ]] || return 1
    while ((${#data} % 4)); do data+="="; done
    printf '%s' "$data" | tr '_-' '/+' | base64 -d 2> /dev/null
}
want_keep=false
if [[ $keep == true ]]; then
    want_keep=true
elif [[ $reconfigure == true ]] || ! grep -q '^KEEP_BRIDGE_URL=' "$ENV_FILE"; then
    step "Google Keep (optional)"
    cat << 'EOF'
Todos can also go on a Google Keep checklist. Keep has no official API for
personal accounts, so this signs in the way an Android phone does, and Google
may ask you to sign in again now and then.
You can add it any time later by re-running this with --keep.
EOF
    if tty_ok && [[ $(ask "Connect Google Keep now? (y/N)") == [yY]* ]]; then
        want_keep=true
    else
        env_set KEEP_BRIDGE_URL ""
    fi
fi
if [[ $want_keep == true ]]; then
    step "Google Keep"
    tty_ok || die "connecting Google Keep needs a terminal to paste the Keep code"
    cat << EOF
On your own computer (not this server), get a Keep code:
  Windows:      download keep-signin-windows.exe from https://github.com/$REPO/releases/latest and open it
  Mac or Linux: run  curl -fsSL https://raw.githubusercontent.com/$REPO/main/deploy/keep-signin.sh | bash
Sign in in the window that opens, then copy the code it shows.
EOF
    keep_login=""
    while :; do
        keep_code=$(ask_secret "Keep code (hidden, empty to skip)")
        [[ -n $keep_code ]] || break
        keep_login=$(decode_keep_code "$keep_code") || keep_login=""
        unset keep_code
        keep_email=${keep_login%%$'\n'*}
        keep_token=${keep_login#*$'\n'}
        [[ $keep_email == *@* && -n $keep_token && $keep_token != "$keep_login" ]] && break
        keep_login=""
        echo "That doesn't look like a Keep code (it starts with keep1:). Try again."
    done
    if [[ -z $keep_login ]]; then
        echo "Skipping Google Keep for now."
        env_set KEEP_BRIDGE_URL ""
    elif ! keep_dir=$(keep_files); then
        echo "Couldn't download the keep-bridge files; skipping Google Keep. Re-run with --keep to try again."
        env_set KEEP_BRIDGE_URL ""
    elif ! printf '%s\n%s\n' "$keep_email" "$keep_token" | bash "$keep_dir/keep-bridge-install.sh" "$keep_dir" full; then
        echo "Google Keep didn't connect; continuing without it. Re-run with --keep to try again."
        grep -q '^KEEP_BRIDGE_URL=.' "$ENV_FILE" || env_set KEEP_BRIDGE_URL ""
    fi
    unset keep_login keep_email keep_token
fi
chown "$RUN_USER:$RUN_USER" "$ENV_FILE"
chmod 0600 "$ENV_FILE"

as_service() {
    systemd-run --quiet --wait --collect --pipe \
        -p User="$RUN_USER" -p Group="$RUN_USER" -p WorkingDirectory="$APP_DIR" \
        -p Environment=OWNER_JID=not-linked-yet -p EnvironmentFile="$ENV_FILE" \
        "$APP_DIR/wa-calendar-agent" "$@"
}

step "Checking your setup"
until as_service check; do
    tty_ok || die "setup check failed; fix the above and re-run"
    echo
    echo "Fix the problem above (sharing can take a minute to apply), then press Enter to retry,"
    reply=$(ask "or type 'skip' to continue anyway")
    [[ $reply == skip ]] && break
done

session=$(absolute "$(env_get WHATSAPP_SESSION_DATABASE_PATH)")
if [[ $relink == true || ! -f $session || -z $(env_get OWNER_JID) ]]; then
    step "Linking WhatsApp"
    systemctl stop "$SERVICE" 2> /dev/null || true
    if [[ $relink == true && -f $session ]]; then
        for file in "$session" "$session-wal" "$session-shm"; do
            [[ -f $file ]] && mv "$file" "$file.unlinked-$(date -u +%Y%m%dT%H%M%SZ)"
        done
    fi
    tty_ok || die "linking WhatsApp needs a terminal to show the QR code"
    [[ -f $session ]] || echo "Scan the QR code with WhatsApp > Settings > Linked devices > Link a device."
    as_service pair < /dev/null 2>&1 | grep --line-buffered -vE 'TRACE|DEBUG|INFO|WARN' \
        | tee "$work/pair.log" || true
    owner_jid=$(sed -n 's/^OWNER_JID=//p' "$work/pair.log" | tail -1)
    [[ -n $owner_jid ]] || die "WhatsApp linking didn't finish; re-run the installer to try again"
    env_set OWNER_JID "$owner_jid"
fi

step "Starting $SERVICE"
systemctl enable "$SERVICE" > /dev/null 2>&1
systemctl restart "$SERVICE"
started=$(date '+%Y-%m-%d %H:%M:%S')
for _ in $(seq 30); do
    journalctl -u "$SERVICE" --since "$started" --no-pager -o cat 2> /dev/null \
        | grep -q "WhatsApp connected" && break
    systemctl is-active --quiet "$SERVICE" || break
    sleep 1
done
if journalctl -u "$SERVICE" --since "$started" --no-pager -o cat | grep -q "WhatsApp connected"; then
    cat << EOF

Done. wa-calendar-agent is running and connected to WhatsApp.
Send "lunch with Abhinav tomorrow at 1pm" to yourself on WhatsApp to try it.

  Logs:      journalctl -u $SERVICE -f
  Settings:  $ENV_FILE (then: systemctl restart $SERVICE)
  Upgrade:   re-run this installer
EOF
    if [[ -n ${backup:-} ]]; then
        echo "  Rollback:  cp -p $backup $APP_DIR/wa-calendar-agent && systemctl restart $SERVICE"
    fi
else
    echo
    journalctl -u "$SERVICE" --since "$started" --no-pager -o cat | tail -20
    die "$SERVICE didn't connect to WhatsApp within 30s; see the logs above"
fi
