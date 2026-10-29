<p align="center">
  <img src="assets/icon.svg" width="88" alt="Postie logo">
</p>

<h1 align="center">Postie</h1>

<p align="center">
  A desktop mail client for Linux that waits in the tray and tells you the moment mail arrives.
</p>

<p align="center">
  <a href="https://github.com/CookieJarTools/Postie/releases/latest"><img src="https://img.shields.io/github/v/release/CookieJarTools/Postie?include_prereleases&label=download&color=5e6ad2&style=flat-square" alt="Latest release"></a>
  <img src="https://img.shields.io/badge/platform-Linux-3a3d45?style=flat-square" alt="Linux">
  <a href="LICENSE.md"><img src="https://img.shields.io/badge/licence-PolyForm%20Shield-3a3d45?style=flat-square" alt="Licence"></a>
</p>

<p align="center">
  <a href="https://github.com/CookieJarTools/Postie/releases/latest"><b>Download</b></a>
  &nbsp;·&nbsp;
  <a href="#features">Features</a>
  &nbsp;·&nbsp;
  <a href="#building-from-source">Build from source</a>
</p>

<br>

![Postie showing an inbox split into Primary and Other, with a message open](docs/screenshots/inbox.png)

<br>

## Install

The easiest way is [The Jar](https://github.com/CookieJarTools/TheJar), which installs Postie and
keeps it up to date.

Or download `Postie-x.y.z.AppImage` from [Releases](https://github.com/CookieJarTools/Postie/releases),
make it executable and run it:

```sh
chmod +x Postie-*.AppImage
./Postie-*.AppImage
```

The first time it opens, it asks for an account. Once one is added, Postie starts when you log in
and sits in the tray. Closing the window doesn't stop it; quit from the tray menu.

### Adding an account

Type your address and Postie fills in the servers for Gmail, Outlook.com, iCloud, Yahoo, Fastmail,
Zoho and GMX. Anything else that speaks IMAP and SMTP works through **Server settings**.

- **Outlook.com and Microsoft 365** sign in with your Microsoft account in the browser. Postie
  keeps a sign-in token, never your password, and you can revoke it from your Microsoft account.
- **Gmail** won't take your normal password over IMAP. Turn on 2-Step Verification, create an app
  password at <https://myaccount.google.com/apppasswords> and paste that in.
- **iCloud, Yahoo and Fastmail** also want an app password from their account settings.

## Features

### Know when mail arrives

New mail shows up within a second or two, because Postie keeps each inbox open with IMAP IDLE
rather than checking on a timer. Each new message gets a desktop notification with **Open**,
**Mark as read** and **Archive** on it. When lots arrive at once you get one summary instead of a
flood. Do Not Disturb is in the tray menu.

### Primary and Other

Like Outlook's Focused Inbox, each inbox is split in two. People you write to, replies to you and
mail marked urgent stay in Primary. Newsletters, mailing lists and automated mail go to Other.
Move any message to the other tab and that sender's mail follows it from then on. Notifications,
the unread badge and the tray dot count Primary only, unless you change that in Settings.

### Fast to get through

Every action is instant: Postie applies it locally and syncs it in the background, so it also
works offline. Keyboard shortcuts cover the common moves, and **Ctrl+K** opens a command menu for
everything else, including every folder.

| | |
| --- | --- |
| J / K | Next / previous message |
| E | Archive |
| # or Delete | Delete |
| R / A / F | Reply / reply all / forward |
| S | Flag |
| U | Mark read or unread |
| C | New message |
| Ctrl+Enter | Send |
| / | Search |
| Ctrl+K | Command menu |
| F5 | Check for new mail |

### Reading

HTML mail opens in a quiet, simplified view with images blocked, so senders can't tell you've
opened it. **Show images** switches to the original: the email on a white page in the sender's own
colours, with its images in place. **Always for** a sender remembers that, the way Outlook does.
**Open in browser** shows the exact original layout. Links show where they really go before you
click them. Attachments open or save from the top of the message, and **Unsubscribe** appears when
the sender offers it. A setting shows every message in a monospace font, if you find plain text
easier to read that way.

### Writing

Reply, reply all and forward (with the original attachments). Addresses autocomplete from people
you've emailed. Each account has its own signature. Drafts save themselves, and **Undo** stays on
screen for a few seconds after you send. Anything that fails to send waits in the Outbox with a
retry.

### Everything else

- All your accounts in one **All inboxes** view, plus Flagged and every folder on the server
- Archive, delete, move, move to Junk, flag, mark read, from the row, the menu or the keyboard
- Search with `from:`, `to:`, `subject:`, `is:unread`, `is:flagged` and `has:attachment`
- Drag accounts in the sidebar to put them in the order you like
- Opens `mailto:` links

## Worth knowing

- **Passwords and tokens** are kept in `~/.config/postie/credentials.json`, readable only by you.
  App passwords and Microsoft tokens can be revoked from your provider without touching your real
  password.
- **Mail** is cached in `~/.local/share/postie/mail.db`. The first sync takes the newest 400
  messages of each folder; **Load older messages** at the bottom of a list fetches more.
- **Linux only** for now. The tray, notifications and start-at-login are written for Linux
  desktops. On Hyprland with the Graphite shell, Postie keeps the desktop from drawing a second
  title bar over its own.
- **Not there yet:** calendar and contacts, rules, conversation threading, and writing HTML mail.

## Building from source

You need Rust 1.88 or newer.

```sh
cargo build --release
./target/release/postie install
```

`install` copies Postie to `~/.local/bin`, adds it to your app menu, turns on start-at-login (as a
systemd user service, or an autostart entry without systemd) and starts it. `--no-login` skips
start-at-login.

| Command | |
| --- | --- |
| `postie` | Open the window, starting the background service if needed |
| `postie compose` | Write a new message |
| `postie mailto:...` | Write to an address |
| `postie sync` | Check every account now |
| `postie quit` | Stop the background service |

To try changes without touching your real mail, run [GreenMail](https://greenmail-mail-test.github.io/greenmail/),
which accepts any login, and point `XDG_CONFIG_HOME` and `XDG_DATA_HOME` at a spare folder:

```sh
podman run -d --rm --name greenmail -p 127.0.0.1:3025:3025 -p 127.0.0.1:3143:3143 \
  -e GREENMAIL_OPTS='-Dgreenmail.setup.test.smtp -Dgreenmail.setup.test.imap -Dgreenmail.hostname=0.0.0.0 -Dgreenmail.auth.disabled' \
  docker.io/greenmail/standalone:2.1.3
```

Then add an account with any address and password, IMAP `127.0.0.1:3143`, SMTP `127.0.0.1:3025`
and security **None**.

`POSTIE_SCREENSHOT=shot.ppm postie window` saves what the window draws to `shot.ppm` after a few
seconds and quits, which is how the screenshot above is made.

### How it's put together

`postie daemon` is the background service: one sync worker per account on its own thread, the tray
icon, notifications, and a control socket. The window is a separate process the service starts on
demand, so closing it frees its memory. Both share one SQLite store; the window queues changes
there and the service replays them on the server.

| | |
| --- | --- |
| `src/daemon/` | The background service: workers, tray, notifications, control socket |
| `src/mail/sync.rs` | Per-account sync, IDLE, replaying queued changes, sending |
| `src/mail/oauth.rs` | Microsoft sign-in |
| `src/mail/parse.rs`, `compose.rs` | Reading messages; building replies, forwards and outgoing mail |
| `src/store.rs` | The store, the change queue, the outbox, drafts, contacts, Primary/Other |
| `src/gui/` | The window |
| `src/autostart.rs` | Menu entry, icon, start-at-login |

## Licence

[PolyForm Shield 1.0.0](LICENSE.md). Use Postie for anything you like; the one thing you may not
do is use it to build a product that competes with it.
