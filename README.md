# devnull

[/dev/null](https://dev-null.sh), the social network for developers, in your terminal.

It follows the web app's layout and shortcuts: the Following, Latest and Trending logs, each commit on the rail in its developer's lane color, threads, profiles, notifications, starred commits, Remotes (direct messages) and a composer that takes images. Written in Rust with [ratatui](https://ratatui.rs).

![devnull in Ghostty: the Latest log, each commit on the rail in its developer's lane color, from their avatar down to its actions](docs/screenshot.png)

## Install

On macOS or Linux:

```bash
curl -fsSL https://raw.githubusercontent.com/mihaidaniel34/dev-null-tui/main/install.sh | sh
```

The script downloads the latest release, checks its SHA-256, and puts `devnull` in `~/.local/bin` (`DEVNULL_INSTALL_DIR` changes that).

Or take a binary from [Releases](https://github.com/mihaidaniel34/dev-null-tui/releases): Linux (static, x86_64 and arm64), macOS (Intel and Apple silicon) and Windows. A macOS binary downloaded in a browser needs `xattr -d com.apple.quarantine devnull` first, as it isn't notarized.

Or build it with Rust 1.85+:

```bash
cargo install --git https://github.com/mihaidaniel34/dev-null-tui
```

## Signing in

**Sign in (recommended).** In Settings (`g s`), press `s`. The terminal is approved as a device, the way the mobile app signs in: devnull makes its own ed25519 key and signs every API request with it. There's no password and no token, and you can revoke the device on the web under Settings → Devices. Signed in, you get every log, notifications, replies, stars, pastéis, quotes and follows.

Approving sends your browser to a `devnull://` link. While you sign in, devnull registers itself for those links, so your browser asks to open "devnull sign-in": allow it, and you're in. If it doesn't ask, paste the link into devnull. Firefox and Safari leave it in the address bar; in Chrome, open DevTools → Network before you approve and copy the `Location` header of the `authorize` request.

**Guest.** Set your GitHub username (`u` in Settings), and devnull builds your Following log from public commits. Guests can still commit and reply, signed with the SSH key on your GitHub account through `ssh-keygen`, exactly like `curl -fsSL https://dev-null.sh/commit | bash`. Agents, passphrases and hardware keys all work.

## What's in it

- **Profiles:** a year of activity in the developer's lane color, their streak, rank and badges (`A` lists every achievement). `⇥` goes through Commits, Replies, Followers and Following.
- **Notifications,** filtered by kind as on the web.
- **Remotes** (`g r`, signed in): the inbox with Open and Stashed tabs, search, and conversations that update on their own. `M` starts one from a profile, a person or a commit's author. In a conversation, `i` writes, `r` replies to a message, `e` reacts and `x` stashes. Orgs, voice notes and sending images are web and app only for now.
- **The composer** suggests developers as you type `@` (`⇥` completes) and keeps a draft you close, even across restarts.
- **Settings:** your lane color, pastéis mode, syncing your profile from GitHub, dark or light (`t`) and icons (`i`).
- **Safety:** `m` mutes and `B` blocks from a profile, and `!` reports a commit.

## Look

The layout fits the terminal. With room to spare (176 × 44 or more) the log widens and avatars grow; as it narrows, or its font grows, the sidebar folds to icons before the right column goes, so the log keeps at least 72 columns.

devnull uses the web's dark and light palettes, picked from your terminal's background, or with `t` in Settings.

![devnull in light mode, in Ghostty](docs/light.png)

- **Avatars and images** show up in [kitty](https://sw.kovidgoyal.net/kitty/) and [Ghostty](https://ghostty.org), through the kitty graphics protocol, and scroll like text. `v` opens a commit's images full size. In tmux, set `allow-passthrough on`.
- **Icons** are the web's, from the Nerd Font symbols that Ghostty, kitty and WezTerm ship. Elsewhere devnull uses plain symbols, unless you switch to Nerd Font icons with `i` in Settings.
- **Big text:** in kitty 0.40+, names, headings and the wordmark are drawn larger, with kitty's [text sizing protocol](https://sw.kovidgoyal.net/kitty/text-sizing-protocol/).
- **Other themes** (Windows 98, and your own TOML files in `themes/` in the config dir) are taking a break while the redesign settles. `DEVNULL_THEME` still picks one; [`themes/dark.toml`](themes/dark.toml) lists every color a theme can set.

## Committing

`c` opens the composer, and `⌃S` commits. To attach images, drop their files on the composer or paste their paths; `⌃V` attaches the image on your clipboard. Up to 3 JPEG, PNG or WebP images, 10 MB and 6000 pixels a side each. A commit with images is always signed with your GitHub SSH key, as the signed API doesn't take images yet.

From the shell:

```bash
devnull commit 'Shipped it.'
devnull commit --image screenshot.png --reply 0be0130 'Fixed, see the screenshot.'
```

## Keys

These match the web app:

| | |
|---|---|
| `j` / `k` | Next / previous commit |
| `↵` | Open the commit (on an open commit, reply to it) |
| `r` / `q` | Reply / quote |
| `o` | Show or hide replies |
| `c` | Write a commit |
| `y` | Copy the link |
| `/` or `⌃K` | Go to a developer, profile link or commit link |
| `g l` / `g e` / `g p` / `g s` | Log / trending / your profile / settings |
| `?` | All shortcuts |

And in the terminal only:

| | |
|---|---|
| `s` / `p` | Star / give a pastel de nata |
| `a` / `f` | Open the author's profile / follow |
| `v` / `w` | View images / open in your browser |
| `D` / `!` | Delete your commit / report one |
| `⇥` or `1` `2` `3` | Switch tabs and filters |
| `g n` / `g r` / `g b` | Notifications / Remotes / starred |
| `g g` / `G` | Top / bottom |
| `R` | Refresh |
| `esc` / `Q` | Back / quit |

The mouse wheel scrolls; click a commit to select it, and again to open it.

## Configuration

| Variable | |
|---|---|
| `DEVNULL_URL` | The server, `https://dev-null.sh` by default. |
| `DEVNULL_CONFIG_DIR` | Where the config and device key live: `~/.config/devnull`, or `~/Library/Application Support/devnull` on macOS. |
| `DEVNULL_THEME` | `dark`, `light`, `windows-98` or your own theme's id. Unset, it's the Settings choice, which follows the terminal by default. |
| `DEVNULL_ICONS` | `nerd` or `plain`. Unset, it's the Settings choice. |
| `DEVNULL_IMAGES` | `off` turns avatars and images off; `on` tries them in a terminal that isn't detected. |
| `DEVNULL_BIG_TEXT` | `off` keeps text one size; `on` skips asking the terminal. |

## Development

devnull talks to the [/dev/null server](https://github.com/dev-null-sh/dev-null.sh)'s signed API, which has no published spec. [`docs/api-contract.md`](docs/api-contract.md) lists every endpoint and field devnull reads, and `scripts/check-api.sh` (run weekly in CI) shows what changed on the server since.

To release, bump `version` in `Cargo.toml` and push to `main`. The Release workflow builds every platform, tags the commit and publishes the release with checksums.
