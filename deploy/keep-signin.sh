#!/usr/bin/env bash
set -euo pipefail

REPO=${WA_REPO:-life2harsh/whatagent}
command -v python3 > /dev/null || {
    echo "python3 is required. On a Mac, run: xcode-select --install" >&2
    exit 1
}
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

echo "Preparing a temporary Python environment (takes a minute the first time)..." >&2
if [[ $(uname) == Linux ]] && { [[ -n ${DISPLAY:-} ]] || [[ -n ${WAYLAND_DISPLAY:-} ]]; }; then
    if ! python3 -c 'import gi; gi.require_version("WebKit2", "4.1")' 2> /dev/null; then
        if command -v apt-get > /dev/null; then pkgs=(apt-get install -y python3-gi gir1.2-webkit2-4.1)
        elif command -v pacman > /dev/null; then pkgs=(pacman -S --needed --noconfirm python-gobject webkit2gtk-4.1)
        elif command -v dnf > /dev/null; then pkgs=(dnf install -y python3-gobject webkit2gtk4.1)
        else pkgs=(); fi
        if ((${#pkgs[@]})); then
            echo "The sign-in window needs ${pkgs[*]: -2}. Installing (may ask for your password)..." >&2
            sudo "${pkgs[@]}" > /dev/null 2>&1 < /dev/null || true
        fi
    fi
fi
python3 -m venv --system-site-packages "$work/venv"
"$work/venv/bin/pip" install -q --disable-pip-version-check gpsoauth pywebview >&2 \
    || "$work/venv/bin/pip" install -q --disable-pip-version-check gpsoauth >&2

helper=""
if [[ -f ${BASH_SOURCE[0]:-} ]]; then
    helper="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/../keep-bridge/get_master_token.py"
fi
if [[ ! -f $helper ]]; then
    helper=$work/get_master_token.py
    curl -fsSL "https://raw.githubusercontent.com/$REPO/main/keep-bridge/get_master_token.py" -o "$helper"
fi
"$work/venv/bin/python" "$helper" "$@"
