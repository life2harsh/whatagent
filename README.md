# whatagent

text yourself on whatsapp, and it ends up in your google calendar or your todo list. voice notes work too.

## why this exists

i wanted something like the Essential Space on Nothing phones: throw a thought somewhere and get it back when it matters. my sticky notes weren't on the cloud, and i forget a lot of stuff, so i built this.

funny enough, i now have a Nothing phone with Essential Space on it. i still use this though. whatsapp is open all day anyway, and texting yourself is about the lowest-effort way to keep track of things.

## what it does

- "dentist tomorrow at 5" goes straight into google calendar
- "buy earbuds" or "remind me to call abhinav at 6" becomes a todo, and you get a whatsapp reminder when it's time
- voice notes work exactly like text, so you can just talk
- todos can also land on one google keep checklist ("WhatsApp Todos"), if you set that up
- repeating stuff works: "every wednesday at 3pm, nptel assignment" becomes a weekly event
- if a time is vague, it tells you what it understood and waits for a yes or no
- it only listens to your own self-chat (message yourself) and ignores every other chat

## see it in action

a voice note about going back to college turns into a calendar event (after a quick yes), a typed "buy GaN charger for phone" turns into a todo, and both show up in google calendar and on the keep checklist.

<p align="center">
  <img src="docs/screenshots/chat-todos.png" alt="voice notes and texts in the self-chat turning into todos and a calendar event" width="320">
  <img src="docs/screenshots/chat-confirm.png" alt="a vague date gets a yes or no check before it's added, then a todo is added to keep" width="320">
</p>
<p align="center">
  <img src="docs/screenshots/calendar-event.png" alt="the event in google calendar, created by the agent" width="260">
  <img src="docs/screenshots/home-widgets.png" alt="calendar and keep checklist widgets on the home screen" width="260">
  <img src="docs/screenshots/keep-checklist.png" alt="the WhatsApp Todos checklist in google keep" width="260">
</p>

<!-- paste the github.com/user-attachments/assets/... link for whatagent-demo.mp4 on its own line below this comment -->


## how it works

```text
your self-chat -> whatsapp (linked device) -> gemini (event or todo?) -> checks in rust -> google calendar, or your todo list (+ google keep)
```

it's a small rust program that runs on a server and shows up in whatsapp as a linked device, like whatsapp web. gemini reads your message (or listens to your voice note) and works out what you meant. rust then checks the result before anything gets created, so a weird answer from the model can't make a mess of your calendar.

## setting it up

you need:

- a linux server that stays on. the cheapest VPS works, and so does a free oracle cloud one. prebuilt binaries cover x86_64 and ARM64 with glibc 2.35+ (ubuntu 22.04+, debian 12+)
- a free gemini API key from [google ai studio](https://aistudio.google.com/apikey)
- a google service account key, so it can add events to your calendar. [docs/GOOGLE_SETUP.md](docs/GOOGLE_SETUP.md) walks you through it, takes about two minutes

### one command on the server

copy the key to the server, then run the installer there:

```bash
scp service-account.json you@your-server:
ssh you@your-server
curl -fsSL https://raw.githubusercontent.com/life2harsh/whatagent/main/deploy/install.sh | sudo bash -s -- --service-account ~/service-account.json
```

the installer:

1. downloads the latest release for your CPU and checks its checksum
2. creates the `wa-calendar` user and `/opt/wa-calendar-agent`, and installs the systemd service
3. asks for your gemini key (and checks it works), your calendar ID and your timezone
4. tells you who to share your calendar with, then tests gemini and calendar access
5. shows a QR code: scan it under whatsapp > settings > linked devices
6. asks you to message yourself once, so it knows which chat is yours
7. starts the service and waits until it's connected

you can delete `~/service-account.json` after that. the installer keeps its own copy, readable only by the service.

### or from your own machine

`deploy/deploy.sh` does the same thing over SSH and uploads the key for you:

```bash
deploy/deploy.sh you@your-server --service-account ./service-account.json
deploy/deploy.sh you@your-server -i ~/.ssh/my-key     # upgrade later
```

add `--binary path/to/wa-calendar-agent` to deploy a binary you built yourself instead of a release.

### google keep (optional)

to also get todos on a google keep checklist, run this from your own machine after the install:

```bash
deploy/keep-setup.sh you@your-server -i ~/.ssh/my-key
```

it opens a small google sign-in window, sets up the keep bridge on the server and connects everything. details are in [docs/GOOGLE_SETUP.md](docs/GOOGLE_SETUP.md).

### upgrading and changing settings

run the same command again to upgrade. your settings, the whatsapp link and your todos are kept, the old binary is backed up next to the new one, and the rollback command is printed at the end. other options:

| option | what it does |
| --- | --- |
| `--version v0.2.0` | install a specific release instead of the latest |
| `--reconfigure` | ask for the gemini key, calendar, timezone and service account again |
| `--relink` | unlink whatsapp and show a new QR code |

settings live in `/etc/wa-calendar-agent.env` (after editing it, run `sudo systemctl restart wa-calendar-agent`). handy commands:

```bash
journalctl -u wa-calendar-agent -f                         # logs
sudo systemctl restart wa-calendar-agent                   # restart
sudo -u wa-calendar sh -c 'set -a; . /etc/wa-calendar-agent.env; /opt/wa-calendar-agent/wa-calendar-agent check'
```

the service is capped at 25% CPU and 512 MB of memory, so it's happy sharing a small VPS with other stuff.

## commands

mostly you just write normally. there are a few exact commands too:

```text
todo submit DBMS assignment | due 2026-09-18T17:00:00+05:30 | remind 2026-09-18T16:00:00+05:30 | note upload the PDF
tasks
done 1
remind 2026-09-18T16:00:00+05:30 | call Abhinav
```

`tasks` lists your open todos, `done 1` ticks one off (and cancels its reminder), and `remind` sets a plain reminder with no todo attached. the `todo` and `remind` commands take exact times with a timezone offset; if you'd rather say "tomorrow at 6", just write it normally.

## details

**reminders.** the service checks for due reminders every `REMINDER_POLL_SECONDS` seconds and sends them to your self-chat. if a send fails, it's retried instead of being dropped.

**google keep.** with `KEEP_BRIDGE_URL` set, every new todo is added as an unchecked item to one checklist (named by `KEEP_LIST_TITLE`, default "WhatsApp Todos") through the small `keep-bridge` service. syncing to keep is retried separately from creating the todo, so a keep hiccup never loses a todo. after changing the bridge code, redeploy it with `deploy/keep-setup.sh you@server -i key --update` (no new sign-in). ticking an item in keep doesn't mark the todo done here yet.

**calendar.** agenda text, notes and other details go into the event description. repeating events use validated RFC 5545 rules, and vague repetition ("sometimes on weekends") is rejected rather than guessed. messages that clearly aren't events or todos (links, greetings, questions) are politely declined.

**no duplicates.** every whatsapp message ID is claimed in SQLite before anything talks to google, so a replayed message is never processed twice. calendar events also get an ID based on the message ID, so even a retry after a lost response can't create a second event.

**state.** SQLite holds processing state, pending confirmations, todos and the delivery queues. it isn't a message archive. the whatsapp session lives in a separate database; treat that file like a password.

## developing locally

1. copy `.env.example` to `.env` and fill in the required values (see below)
2. set up gemini and the calendar service account as in [docs/GOOGLE_SETUP.md](docs/GOOGLE_SETUP.md)
3. try it without whatsapp:

```bash
cargo run -- test-calendar "cal OSDC meeting tomorrow at 6pm"
```

this calls gemini and google calendar for real (it creates a real event) and prints the reply you'd get on whatsapp.

to link whatsapp and find your `OWNER_JID`, set `WHATSAPP_SESSION_DATABASE_PATH`, run `cargo run -- pair`, scan the QR code and message yourself once. it prints the exact `OWNER_JID=` line to use and exits. after that, `cargo run -- run` starts the daemon, and `cargo run -- check` tests the gemini key and calendar access without sending anything.

**environment.** see `.env.example`. `OWNER_JID`, `GEMINI_API_KEY`, `GOOGLE_SERVICE_ACCOUNT_KEY_PATH` and `GOOGLE_CALENDAR_ID` are required; `KEEP_BRIDGE_URL` and `KEEP_BRIDGE_TOKEN` turn on keep sync. don't commit `.env`, the session database or any key files.

checks:

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
cargo build --release
```

### publishing releases

the installer downloads binaries from this repo's github releases, so the repo has to be public. push a version tag to build and publish them:

```bash
git tag v0.1.0
git push origin v0.1.0
```

`.github/workflows/release.yml` builds x86_64 and ARM64 binaries and attaches them to the release. forks can point the installer at their own releases with `WA_REPO=owner/repo`.

### full-pipeline checklist

- [ ] the service account's email has "make changes to events" on `GOOGLE_CALENDAR_ID`
- [ ] `cargo run -- test-calendar "cal test event tomorrow at 6pm"` creates an event
- [ ] a deliberately vague request asks for confirmation, and `yes` creates exactly one event
- [ ] `OWNER_JID` matches the line printed by `wa-calendar-agent pair` (the installer does this for you)
- [ ] after `cargo run -- run` and scanning the QR code, the session database exists and only you can read it
- [ ] `cal test event tomorrow at 6pm` in your self-chat gets one short reply
- [ ] a voice note for an event creates one calendar event and one whatsapp confirmation
- [ ] a voice note like "remind me to buy milk tomorrow at 6pm" creates one todo (and one new keep checklist item, if keep is on)
- [ ] a clear repeating request shows up as a repeating event in google calendar
- [ ] add a todo, list it, complete it, and check that one scheduled reminder reaches your self-chat
