#!/usr/bin/env bash
set -euo pipefail

usage() {
    cat << 'EOF'
Sets up the Google side of wa-calendar-agent from Google Cloud Shell, so you
don't have to click through the Cloud Console. Open https://shell.cloud.google.com
and run:

  curl -fsSL https://raw.githubusercontent.com/life2harsh/whatagent/main/deploy/google-setup.sh | bash

It creates (or reuses) a Google Cloud project called "whatagent", then prints:
  - a Gemini API key, for the installer's Gemini question
  - a calendar code, for the installer's Google Calendar question
The calendar code holds a service account key: treat it like a password.
Running it again reuses the project, service account and Gemini key, and
makes a fresh calendar code. Nothing here needs billing.

Options:
  --calendar-id ID   calendar to use (default: your main calendar)
EOF
    exit 0
}

NAME=whatagent
export CLOUDSDK_CORE_DISABLE_PROMPTS=1
say() { printf '%s\n' "$*" >&2; }
die() { printf '\033[31merror:\033[0m %s\n' "$*" >&2; exit 1; }
step() { printf '\n\033[1m==> %s\033[0m\n' "$*" >&2; }
retry() {
    local attempt
    for attempt in 1 2 3 4 5 6; do
        "$@" && return 0
        ((attempt < 6)) && sleep $((attempt * 5))
    done
    return 1
}

calendar_id=""
while (($#)); do
    case $1 in
        --calendar-id) calendar_id=${2:?--calendar-id needs a value}; shift 2 ;;
        -h | --help) usage ;;
        *) die "unknown option: $1" ;;
    esac
done

command -v gcloud > /dev/null || die "gcloud isn't here. Run this in Google Cloud Shell: https://shell.cloud.google.com"
account=$(gcloud config get-value account 2> /dev/null || true)
[[ -n $account ]] || die "you aren't signed in to gcloud. Open https://shell.cloud.google.com and run this there."
calendar_id=${calendar_id:-$account}

step "Google Cloud project"
project=$(gcloud projects list --filter="name=$NAME AND lifecycleState=ACTIVE" \
    --format='value(projectId)' --limit=1 2> /dev/null || true)
if [[ -n $project ]]; then
    say "Using your existing project $project."
else
    project=$NAME-$(printf '%06d' $(((RANDOM * 32768 + RANDOM) % 1000000)))
    say "Creating project $project..."
    gcloud projects create "$project" --name="$NAME" > /dev/null \
        || die "couldn't create a project. Work and school accounts may not be allowed to; try a personal Gmail account."
fi

step "Turning on the Google APIs (takes a minute)"
retry gcloud services enable calendar-json.googleapis.com generativelanguage.googleapis.com \
    apikeys.googleapis.com --project="$project" > /dev/null \
    || die "couldn't turn on the APIs for $project"

step "Gemini API key"
gemini_key=""
key_name=$(gcloud services api-keys list --project="$project" --filter="displayName=$NAME" \
    --format='value(name)' --limit=1 2> /dev/null || true)
if [[ -z $key_name ]]; then
    retry gcloud services api-keys create --project="$project" --display-name="$NAME" \
        --api-target=service=generativelanguage.googleapis.com > /dev/null 2>&1 || true
    key_name=$(gcloud services api-keys list --project="$project" --filter="displayName=$NAME" \
        --format='value(name)' --limit=1 2> /dev/null || true)
fi
[[ -n $key_name ]] && gemini_key=$(gcloud services api-keys get-key-string "$key_name" \
    --format='value(keyString)' 2> /dev/null || true)
[[ -n $gemini_key ]] || say "Couldn't make a Gemini key here; get one at https://aistudio.google.com/apikey instead."

step "Service account for Google Calendar"
sa_email=$NAME@$project.iam.gserviceaccount.com
if ! gcloud iam service-accounts describe "$sa_email" --project="$project" > /dev/null 2>&1; then
    gcloud iam service-accounts create "$NAME" --project="$project" --display-name="$NAME" > /dev/null \
        || die "couldn't create the service account"
fi
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
retry gcloud iam service-accounts keys create "$work/key.json" --iam-account="$sa_email" \
    --project="$project" > /dev/null 2>&1 \
    || die "couldn't create a key for $sa_email. Work and school accounts may block this; try a personal Gmail account."
calendar_code=cal1:$( { printf '%s\n' "$calendar_id"; cat "$work/key.json"; } | gzip -9c | base64 -w0 \
    | tr '+/' '-_' | tr -d '=')

step "Done"
cat << EOF

Now run the installer on your server and paste these when it asks.

Gemini API key:
${gemini_key:-(get one at https://aistudio.google.com/apikey)}

Calendar code (treat it like a password):
$calendar_code

One more step, in Google Calendar (on a computer, not the phone app):
  1. Open https://calendar.google.com/calendar/r/settings
  2. On the left, under "Settings for my calendars", click your calendar
  3. "Share with specific people or groups" > "Add people and groups"
  4. Add $sa_email
     with "Make changes to events", then click Send
This lets whatagent add events to $calendar_id.
EOF
