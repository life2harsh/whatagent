#!/usr/bin/env bash
# Installs or upgrades wa-calendar-agent on a server over SSH, from your machine.
#
#   deploy/deploy.sh user@host [options]
#
# Options:
#   -i, --ssh-key FILE       SSH private key for the server
#   --service-account FILE   upload this Google service account JSON key
#   --binary FILE            upload a binary you built instead of downloading a release
#   --version TAG            release to install (default: latest)
#   --reconfigure            ask for every setting again
#   --relink                 link WhatsApp again (new QR code)
set -euo pipefail

usage() { sed -n '2,13p' "$0" | sed 's/^# \{0,1\}//'; exit "${1:-0}"; }

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
        --reconfigure | --relink) forward+=("$1"); shift ;;
        -h | --help) usage ;;
        -*) echo "unknown option: $1" >&2; usage 2 ;;
        *) [[ -z $host ]] || usage 2; host=$1; shift ;;
    esac
done
[[ -n $host ]] || usage 2
[[ -z $service_account || -f $service_account ]] || { echo "file not found: $service_account" >&2; exit 1; }
[[ -z $binary || -f $binary ]] || { echo "file not found: $binary" >&2; exit 1; }

# A private (0700) staging directory on the server, removed afterwards.
remote=$(ssh "${ssh_opts[@]}" "$host" 'mktemp -d')
trap 'ssh "${ssh_opts[@]}" "$host" "rm -rf $remote" || true' EXIT

echo "Uploading to $host..."
scp -q "${ssh_opts[@]}" "$here/install.sh" "$here/wa-calendar-agent.service" "$host:$remote/"
if [[ -n $binary ]]; then
    scp -q "${ssh_opts[@]}" "$binary" "$host:$remote/wa-calendar-agent"
fi
if [[ -n $service_account ]]; then
    scp -q "${ssh_opts[@]}" "$service_account" "$host:$remote/service-account.json"
    forward+=(--service-account "$remote/service-account.json")
fi

ssh -t "${ssh_opts[@]}" "$host" "sudo bash $(printf '%q ' "$remote/install.sh" "${forward[@]}")"
