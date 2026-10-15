#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat << 'EOF'
Installs or upgrades wa-calendar-agent on a server over SSH, from your machine.

  deploy/deploy.sh user@host [options]

Options:
  -i, --ssh-key FILE       SSH private key for the server
  --calendar               connect (or change) Google Calendar
  --keep                   connect (or reconnect) Google Keep
  --service-account FILE   upload this Google service account JSON key
  --binary FILE            upload a binary you built instead of downloading a release
  --version TAG            release to install (default: latest)
  --reconfigure            ask for every setting again
  --relink                 link WhatsApp again (new QR code)
EOF
    exit "${1:-0}"
}

here=$(cd "$(dirname "$0")" && pwd)
host=""
ssh_opts=(-o ConnectTimeout=15)
forward=()
service_account=""
binary=""
while (($#)); do
    case $1 in
        -i | --ssh-key) ssh_opts+=(-i "${2:?$1 needs a file}"); shift 2 ;;
        --service-account) service_account=${2:?$1 needs a file}; shift 2 ;;
        --binary) binary=${2:?$1 needs a file}; shift 2 ;;
        --version) forward+=("$1" "${2:?$1 needs a tag}"); shift 2 ;;
        --reconfigure | --relink | --calendar | --keep) forward+=("$1"); shift ;;
        -h | --help) usage ;;
        -*) echo "unknown option: $1" >&2; usage 2 ;;
        *) [[ -z $host ]] || usage 2; host=$1; shift ;;
    esac
done
[[ -n $host ]] || usage 2
[[ -z $service_account || -f $service_account ]] || { echo "file not found: $service_account" >&2; exit 1; }
[[ -z $binary || -f $binary ]] || { echo "file not found: $binary" >&2; exit 1; }

remote=$(ssh "${ssh_opts[@]}" "$host" 'mktemp -d')
trap 'ssh "${ssh_opts[@]}" "$host" "rm -rf $remote" || true' EXIT

echo "Uploading to $host..."
scp -q "${ssh_opts[@]}" "$here/install.sh" "$here/wa-calendar-agent.service" "$host:$remote/"
# shellcheck disable=SC2029
ssh "${ssh_opts[@]}" "$host" "mkdir $remote/keep-bridge"
scp -q "${ssh_opts[@]}" "$here/../keep-bridge/keep_bridge.py" "$here/../keep-bridge/requirements.txt" \
    "$here/keep-bridge.service" "$here/keep-bridge-install.sh" "$host:$remote/keep-bridge/"
if [[ -n $binary ]]; then
    scp -q "${ssh_opts[@]}" "$binary" "$host:$remote/wa-calendar-agent"
fi
if [[ -n $service_account ]]; then
    scp -q "${ssh_opts[@]}" "$service_account" "$host:$remote/service-account.json"
    forward+=(--service-account "$remote/service-account.json")
fi

ssh -t "${ssh_opts[@]}" "$host" "sudo bash $(printf '%q ' "$remote/install.sh" "${forward[@]}")"
