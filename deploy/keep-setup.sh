#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat << 'EOF'
Turns on Google Keep sync for wa-calendar-agent, from your machine.

  deploy/keep-setup.sh user@host [-i ssh-key]
  deploy/keep-setup.sh user@host [-i ssh-key] --update

Default: walks you through a Google sign-in, gets a Keep master token,
installs keep-bridge on the server, checks it can log in, and points the
agent at it. The token travels over ssh stdin; it is never printed or
written to disk on this machine. Re-run it to replace an expired token.
With --update: redeploys the keep-bridge code only, reusing the token already
on the server (no sign-in). Use it after changing the bridge.
EOF
    exit "${1:-0}"
}

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

email=""
master_token=""

if [[ $mode == full ]]; then
    login=$(bash "$here/keep-signin.sh" --env)
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

remote=$(ssh "${ssh_opts[@]}" "$host" 'mktemp -d')
echo "Uploading keep-bridge to $host..."
scp -q "${ssh_opts[@]}" "$here/../keep-bridge/keep_bridge.py" "$here/../keep-bridge/requirements.txt" \
    "$here/keep-bridge.service" "$here/keep-bridge-install.sh" "$host:$remote/"
# shellcheck disable=SC2029
if [[ $mode == full ]]; then
    printf '%s\n%s\n' "$email" "$master_token" \
        | ssh "${ssh_opts[@]}" "$host" "sudo bash $remote/keep-bridge-install.sh $remote full; rc=\$?; sudo rm -rf $remote; exit \$rc"
else
    ssh "${ssh_opts[@]}" "$host" "sudo bash $remote/keep-bridge-install.sh $remote update; rc=\$?; sudo rm -rf $remote; exit \$rc"
fi
echo
if [[ $mode == full ]]; then
    echo "Done. Send \"todo test keep sync\" to yourself on WhatsApp; it should show up in Google Keep."
else
    echo "Done. The keep-bridge code is updated and restarted."
fi
