#!/usr/bin/env bash
# Installs or upgrades wa-calendar-agent on a Linux server with systemd.
#
#   curl -fsSL https://raw.githubusercontent.com/life2harsh/whatagent/main/deploy/install.sh | sudo bash
#
# The first run asks for your Gemini key, calendar and service account key,
# shows a WhatsApp QR code, and starts the service. Re-running it upgrades the
# binary and keeps your settings and WhatsApp session.
#
# Options:
#   --service-account FILE  Google service account JSON key to install
#   --binary FILE           install this binary instead of downloading a release
#   --version TAG           release to download (default: latest)
#   --reconfigure           ask for every setting again
#   --relink                link WhatsApp again (new QR code)
set -euo pipefail

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
while (($#)); do
    case $1 in
        --service-account) service_account=${2:?--service-account needs a file}; shift 2 ;;
        --binary) binary=${2:?--binary needs a file}; shift 2 ;;
        --version) version=${2:?--version needs a tag}; shift 2 ;;
        --reconfigure) reconfigure=true; shift ;;
        --relink) relink=true; shift ;;
        -h | --help) sed -n '2,16p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
done

step() { printf '\n\033[1m==> %s\033[0m\n' "$*"; }
die() { printf '\033[31merror:\033[0m %s\n' "$*" >&2; exit 1; }

[[ $EUID -eq 0 ]] || die "run this as root (sudo bash install.sh)"
command -v systemctl > /dev/null || die "systemd is required"
command -v curl > /dev/null || die "curl is required (apt install curl)"
umask 077

# Prompts read from the terminal so the script also works through curl | bash.
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

env_get() { [[ -f $ENV_FILE ]] && sed -n "s/^$1=//p" "$ENV_FILE" | tail -1 || true; }
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

# When run from a release bundle or a checkout, use the files next to the script.
script_dir=""
[[ -f ${BASH_SOURCE[0]:-} ]] && script_dir=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
[[ -z $binary && -n $script_dir && -x $script_dir/wa-calendar-agent ]] && binary=$script_dir/wa-calendar-agent

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
    # Keep the three most recent backups (timestamped names sort by age).
    backups=("$APP_DIR"/wa-calendar-agent.old-*)
    if ((${#backups[@]} > 3)); then rm -f "${backups[@]:0:${#backups[@]}-3}"; fi
fi
install -m 0755 "$binary" "$APP_DIR/wa-calendar-agent.new"
mv -f "$APP_DIR/wa-calendar-agent.new" "$APP_DIR/wa-calendar-agent"
install -m 0644 "$unit" "$UNIT_FILE"
systemctl daemon-reload

# Settings. Existing values are kept unless --reconfigure is passed.
[[ -f $ENV_FILE ]] || install -m 0600 -o "$RUN_USER" -g "$RUN_USER" /dev/null "$ENV_FILE"
[[ -n $(env_get RUST_LOG) ]] || env_set RUST_LOG info
[[ -n $(env_get DATABASE_URL) ]] || env_set DATABASE_URL "sqlite://$APP_DIR/wa-calendar.db"
[[ -n $(env_get WHATSAPP_SESSION_DATABASE_PATH) ]] \
    || env_set WHATSAPP_SESSION_DATABASE_PATH "$APP_DIR/wa-session.db"

key_path=$(env_get GOOGLE_SERVICE_ACCOUNT_KEY_PATH)
key_path=$(absolute "${key_path:-$APP_DIR/service-account.json}")
if [[ -n $service_account || $reconfigure == true || ! -f $key_path ]]; then
    step "Google service account"
    if [[ -z $service_account ]]; then
        echo "Create one with docs/GOOGLE_SETUP.md, then copy its JSON key to this server."
        service_account=$(ask "Path to the service account JSON key on this server")
    fi
    service_account=${service_account/#\~/$HOME}
    [[ -f $service_account ]] || die "file not found: $service_account"
    grep -q '"type": *"service_account"' "$service_account" \
        || die "$service_account is not a service account key"
    key_path=$APP_DIR/service-account.json
    install -m 0600 -o "$RUN_USER" -g "$RUN_USER" "$service_account" "$key_path"
    env_set GOOGLE_SERVICE_ACCOUNT_KEY_PATH "$key_path"
fi
sa_email=$(sed -n 's/.*"client_email": *"\([^"]*\)".*/\1/p' "$key_path" | head -1)

if [[ $reconfigure == true || -z $(env_get GEMINI_API_KEY) ]]; then
    step "Gemini"
    echo "Get a free API key at https://aistudio.google.com/apikey"
    while :; do
        gemini_key=$(ask_secret "Gemini API key (hidden)")
        [[ -n $gemini_key ]] || continue
        # Sent as a header on stdin so the key never shows up in the process list.
        if printf 'x-goog-api-key: %s\n' "$gemini_key" | curl -fsS -o /dev/null -H @- \
            "https://generativelanguage.googleapis.com/v1beta/models?pageSize=1" 2> /dev/null; then
            break
        fi
        echo "Gemini rejected that key. Try again."
    done
    env_set GEMINI_API_KEY "$gemini_key"
    unset gemini_key
fi

calendar_changed=false
if [[ $reconfigure == true || -z $(env_get GOOGLE_CALENDAR_ID) ]]; then
    step "Google Calendar"
    calendar_id=$(ask "Calendar ID (your Gmail address for your main calendar)" "$(env_get GOOGLE_CALENDAR_ID)")
    [[ -n $calendar_id ]] || die "a calendar ID is required"
    env_set GOOGLE_CALENDAR_ID "$calendar_id"
    calendar_changed=true
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

if [[ $calendar_changed == true ]]; then
    cat << EOF

Share your calendar with the service account:
  Google Calendar > Settings > $(env_get GOOGLE_CALENDAR_ID) > Share with specific people
  Add: ${sa_email:-the client_email from the key file}
  Permission: "Make changes to events"
EOF
    [[ -n ${sa_email:-} ]] && echo "Also enable the Calendar API: https://console.cloud.google.com/apis/library/calendar-json.googleapis.com?project=$(sed -n 's/.*"project_id": *"\([^"]*\)".*/\1/p' "$key_path" | head -1)"
    tty_ok && ask "Press Enter once that's done" > /dev/null
fi
chown "$RUN_USER:$RUN_USER" "$ENV_FILE"
chmod 0600 "$ENV_FILE"

# Runs a subcommand as the service user, reading the env file exactly like the
# service does. OWNER_JID gets a placeholder until WhatsApp is linked (values
# in the env file take precedence).
as_service() {
    systemd-run --quiet --wait --collect --pipe \
        -p User="$RUN_USER" -p Group="$RUN_USER" -p WorkingDirectory="$APP_DIR" \
        -p Environment=OWNER_JID=not-linked-yet -p EnvironmentFile="$ENV_FILE" \
        "$APP_DIR/wa-calendar-agent" "$@"
}

step "Checking Gemini and Google Calendar"
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
    # Hide routine logs so the QR code and instructions stand out.
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
