# devnull

[/dev/null](https://dev-null.sh), the social network for developers, in your terminal.

It follows the web app's layout and keyboard shortcuts: the Following, Latest and Trending logs, the commit graph rail in each developer's lane color, threads, profiles, notifications, starred commits, Remotes (direct messages) and a composer that takes images. In kitty and Ghostty, avatars and images show up too. It's written in Rust with [ratatui](https://ratatui.rs).

![devnull in Ghostty: the Latest log, each commit on the rail in its developer's lane color, from their avatar down to its actions](docs/screenshot.png)

## Install

On macOS or Linux:

```bash
curl -fsSL https://raw.githubusercontent.com/mihaidaniel34/dev-null-tui/main/install.sh | sh
```

The script downloads the latest release for your platform, checks its SHA-256, and puts `devnull` in `~/.local/bin` (set `DEVNULL_INSTALL_DIR` to change that).

Or grab a binary from [Releases](https://github.com/mihaidaniel34/dev-null-tui/releases): Linux (static, x86_64 and arm64), macOS (Intel and Apple silicon) and Windows. A macOS binary downloaded through a browser needs `xattr -d com.apple.quarantine devnull` before it runs, because it isn't notarized.

Or build it with Rust 1.85+:

```bash
cargo install --git https://github.com/mihaidaniel34/dev-null-tui
```

To commit without signing in, you also need `ssh-keygen`, which ships with OpenSSH.

## Two ways in

**Sign in (recommended).** In Settings (`g s`), press `s`. This approves the terminal as a device, the same way the mobile app signs in: the TUI makes its own ed25519 key, opens `/mobile/authorize` with a PKCE challenge, and signs every API request with that key (SSHSIG, namespace `devnull-api`). There's no password and no token. Signing in gives you every log, notifications, replies, stars, pastéis, quotes and follows.

When you approve the device, the page sends your browser to a `devnull://auth/callback?code=…` link meant for the mobile app. While you sign in, devnull registers itself as the handler for `devnull://`: a small hidden app on macOS, a `.desktop` entry on Linux, or a per-user registry key on Windows. Your browser asks to open "devnull sign-in". Allow it, and the TUI finishes signing in on its own. Then it removes the handler, so `devnull://` goes back to the app if you have it.

If your browser doesn't ask, paste the link into the TUI instead. Firefox and Safari leave it in the address bar. In Chrome, open DevTools → Network before you approve, then copy the `Location` header of the `authorize` request.

You can revoke the device on the web under Settings → Devices.

**Guest.** Set your GitHub username (`u` in Settings). The TUI builds your Following log from the public commits of the people you follow, through the public API. Guests can still commit and reply: the TUI signs with the SSH key on your GitHub account through `ssh-keygen -Y sign -n devnull-commit`, exactly like `curl -fsSL https://dev-null.sh/commit | bash`. Your SSH agent, passphrase-protected keys and hardware keys all work. When ssh-keygen needs a passphrase, the TUI steps aside so you can type it.

## Around the app

![A profile in kitty: the activity graph in the developer's lane color, their streak, rank and badges, and their commits](docs/profile.png)

- **Profiles** show a year of activity as a contribution graph in the developer's lane color (or bars for a newer account), their streak, Unix rank and badges. `A` lists every achievement with its progress. `⇥` moves through Commits, Replies, Followers and Following.
- **Notifications** filter by kind, as on the web: replies, mentions, quotes, stars, follows and moderation.
- **Settings** (`g s`) picks your color from the six lanes, Automatic or any hex color, toggles pastéis mode, syncs your profile from GitHub, picks dark or light (`t`), and switches icons (`i`).
- **The composer** suggests developers as you type `@`, and `⇥` completes. A draft you close is kept, even across restarts.
- **Code blocks** are highlighted the way the web highlights them, when you're signed in.
- **Safety:** `m` mutes and `B` blocks someone from their profile, and `!` reports a commit to the moderators.

## Remotes

Remotes, /dev/null's direct messages, are in the sidebar (`g r`) when you're signed in. The inbox has Open and Stashed tabs, a search for names and `@handles`, and a count of what you haven't pulled yet. `⏎` opens a conversation.

- **Starting one:** `M` on someone's profile (there's a Remote button next to Follow), on a person in a list, or on a commit's author opens a conversation with them. Searching the inbox with `/` also lists people you haven't talked to yet under "Start a conversation", and `n` opens a picker to search everyone. It says why if you can't push to someone yet.
- **In the inbox:** `x` stashes or unstashes, `/` searches, `⇥` switches tabs, `S` turns Share pulls on or off, `a` opens their profile.
- **In a conversation:** `j`/`k` pick a message, `i` or `⏎` writes (`⏎` pushes, `alt ⏎` adds a line, `esc` stops and keeps the draft), `r` replies to the picked message, `e` then `1`–`6` reacts, `y` copies it, `v` views its images, `x` stashes. Your messages sit on the right, theirs on the left, with reactions, the time under each run and, when you both share pulls, when they pulled.
- It polls while it's open, so new messages and reactions appear on their own. A conversation that's closed to you (a request you haven't merged, someone who doesn't follow you, Remotes turned off) says so instead of opening the composer.

Not in the terminal yet: orgs, voice notes, and sending images. Open those on the web or in the app.

## When the server changes

devnull talks to the signed API of the [/dev/null server](https://github.com/dev-null-sh/dev-null.sh), which has no published spec. [`docs/api-contract.md`](docs/api-contract.md) lists every endpoint devnull uses and the fields it reads, `scripts/check-api.sh` shows what changed in those server files since the contract was last checked, and a weekly workflow runs it. The types in `src/model.rs` ignore fields they don't know and default the ones that go missing, so most changes cost nothing.

## Look

The layout fits the terminal. With room to spare (176 columns by 44 rows or more), the log widens and avatars grow. As the terminal narrows, or its font grows, the sidebar folds to icons before the right column goes, so the log keeps at least 72 columns.

devnull uses the web's dark and light palettes. It asks your terminal for its background color to pick one, and `t` in Settings goes through auto, dark and light.

![devnull in light mode, in Ghostty](docs/light.png)

**Icons.** In Ghostty, kitty and WezTerm, which ship the Nerd Font symbols, devnull uses the web's outline icons. Elsewhere it uses plain symbols, unless you have a Nerd Font: `i` in Settings goes through auto, Nerd Font and plain, and `DEVNULL_ICONS` sets it from the environment.

**Big text.** In kitty 0.40 or later, names, headings and the wordmark are drawn larger, with kitty's [text sizing protocol](https://sw.kovidgoyal.net/kitty/text-sizing-protocol/). devnull asks the terminal whether it can, so everywhere else text stays one size. `DEVNULL_BIG_TEXT=off` turns it off.

**Other themes** are taking a break while the redesign settles, so Settings only offers dark and light. `DEVNULL_THEME` still picks one: `light`, `dark`, `windows-98`, or a TOML file of yours in `themes/` in the config dir, named after its id. [`themes/dark.toml`](themes/dark.toml) lists every color a theme can set, and a theme only needs the ones it changes:

```toml
# ~/.config/devnull/themes/solarized.toml
name = "Solarized"
extends = "dark"

[colors]
shell = "#002b36"
ink = "#93a1a1"
primary = "#268bd2"
```

## Images

To attach an image, drop its file onto the composer, or paste its path (a terminal pastes a dropped file as its path). `⌃V` attaches the image on your clipboard, such as a screenshot, and `⌃X` removes the last one. A commit carries up to 3 JPEG, PNG or WebP images, of up to 10 MB and 6000 pixels a side.

The signed API doesn't take images yet, so a commit with images is always signed with the SSH key on your GitHub account, the way the `/commit` script sends them. That works signed in or not. A quote can't carry images yet.

## Avatars and images in the terminal

In [kitty](https://sw.kovidgoyal.net/kitty/) and [Ghostty](https://ghostty.org), devnull shows avatars, cut round with a ring in each developer's lane color, and the images in commits. It uses the kitty graphics protocol's Unicode placeholders, so pictures scroll and redraw like text. Inside tmux, this needs `set -g allow-passthrough on`. Other terminals keep the colored dots.

`v` opens a commit's images full size. Set `DEVNULL_IMAGES=off` to turn pictures off, or `DEVNULL_IMAGES=on` to try them in another terminal that supports the protocol's Unicode placeholders.

## From the shell

```bash
devnull commit 'Shipped it.'
devnull commit --image screenshot.png --reply 0be0130 'Fixed, see the screenshot.'
```

A plain commit uses the device key if you're signed in, and your GitHub SSH key if you're not. With `--image` (up to 3) or `--reply` (a commit's hash or link), it always uses your GitHub SSH key.

## Keys

These match the web app:

| | |
|---|---|
| `j` / `k` | Next / previous commit |
| `↵` | Open the commit (on an open commit, reply to it) |
| `y` | Copy the link |
| `r` / `q` | Reply / quote |
| `o` | Show or hide replies |
| `c` | Write a commit |
| `/` or `⌃K` | Go to a developer, profile link or commit link |
| `⌃S` (or `⌃↵`) / `⌃P` | Commit / preview, in the composer |
| `⌃V` / `⌃X` | Attach the clipboard's image / remove the last one, in the composer |
| `⇥` | Complete an @mention, in the composer |
| `g l` / `g e` / `g p` / `g s` | Log / trending / your profile / settings |
| `?` | All shortcuts |

These are extra, terminal only:

| | |
|---|---|
| `s` / `p` | Star / give a pastel de nata (pastéis mode) |
| `v` | View a commit's images full size (kitty, Ghostty) |
| `f` | Follow or unfollow, on a profile or in a list of people |
| `m` / `B` | Mute / block, on a profile |
| `A` | Achievements, on a profile |
| `!` | Report a commit |
| `a` | Open the author's profile |
| `w` | Open in your browser |
| `D` | Delete your commit |
| `⇥` / `1` `2` `3` | Switch tabs and filters |
| `g n` / `g b` / `g g` / `G` | Notifications / starred / top / bottom |
| `R` | Refresh (the log also counts new commits every minute, like the web) |
| `esc` | Back |
| `⌃C` / `Q` | Quit |

The mouse wheel scrolls. Click a commit to select it, and click it again to open it.

## Configuration

| Variable | |
|---|---|
| `DEVNULL_URL` | The server. Defaults to `https://dev-null.sh`. `http://localhost:8000` works for a local checkout. |
| `DEVNULL_CONFIG_DIR` | Where `config.json` and the device key live. Defaults to `~/.config/devnull` (`~/Library/Application Support/devnull` on macOS). |
| `DEVNULL_IMAGES` | `off` turns avatars and images off. `on` turns them on where they aren't detected. |
| `DEVNULL_ICONS` | `nerd` or `plain`. Left unset, it's the setting from Settings, `auto` by default, which picks Nerd Font icons in Ghostty, kitty and WezTerm. |
| `DEVNULL_BIG_TEXT` | `off` keeps text one size. `on` draws big text without asking the terminal first. |
| `DEVNULL_THEME` | A theme's id, like `light`, `dark` or `windows-98`. Left unset, it's the theme from Settings, `auto` by default, which follows the terminal's background. |

Colors use the web's palettes. On terminals without truecolor, they're mapped to the 256-color palette.

## Releasing

Bump `version` in `Cargo.toml` and push to `main`. The Release workflow sees a version with no tag yet, builds every platform, tags the commit and publishes the release with checksums and generated notes. Pushing a `v*` tag that matches `Cargo.toml` works too.
