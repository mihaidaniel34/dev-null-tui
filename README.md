# devnull

[/dev/null](https://dev-null.sh), the social network for developers, in your terminal.

It follows the web app's layout and keyboard shortcuts: the Following, Latest and Trending logs, the commit graph rail in each developer's lane color, threads, profiles, notifications, saved commits and a composer. It's written in Rust with [ratatui](https://ratatui.rs).

![devnull in a terminal: the Latest log, with a quoted commit and a reply branching off the rail](docs/screenshot.png)

<sub>Demo data from a local mock server. The developers in it are made up.</sub>

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

**Sign in (recommended).** In Settings (`g s`), press `s`. This approves the terminal as a device, the same way the mobile app signs in: the TUI makes its own ed25519 key, opens `/mobile/authorize` with a PKCE challenge, and signs every API request with that key (SSHSIG, namespace `devnull-api`). There's no password and no token. Signing in gives you every log, notifications, replies, stars, saves, pastéis, quotes and follows.

When you approve the device, the page redirects your browser to a `devnull://auth/callback?code=…` link meant for the app. Copy that link and paste it into the TUI.
- **Firefox and Safari** leave the link in the address bar.
- **Chrome** may not show the link. If it doesn't, open DevTools → Network before you approve, then copy the `Location` header of the `authorize` request.

You can revoke the device on the web under Settings → Devices.

**Guest.** Set your GitHub username (`u` in Settings). The TUI builds your Following log from the public commits of the people you follow, through the public API. Guests can still commit and reply: the TUI signs with the SSH key on your GitHub account through `ssh-keygen -Y sign -n devnull-commit`, exactly like `curl -fsSL https://dev-null.sh/commit | bash`. Your SSH agent, passphrase-protected keys and hardware keys all work. When ssh-keygen needs a passphrase, the TUI steps aside so you can type it.

## From the shell

```bash
devnull commit 'Shipped it.'
```

This uses the device key if you're signed in, and your GitHub SSH key if you're not.

## Keys

These match the web app:

| | |
|---|---|
| `j` / `k` | Next / previous commit |
| `↵` | Open the commit (or the author, on an open commit) |
| `y` | Copy the link |
| `r` / `q` | Reply / quote |
| `o` | Show or hide replies |
| `c` | Write a commit |
| `/` or `⌃K` | Go to a developer, profile link or commit link |
| `⌃S` (or `⌃↵`) / `⌃P` | Commit / preview, in the composer |
| `g l` / `g e` / `g p` / `g s` | Log / trending / your profile / settings |
| `?` | All shortcuts |

These are extra, terminal only:

| | |
|---|---|
| `s` / `b` / `p` | Star / save / give a pastel de nata (pastéis mode) |
| `f` | Follow or unfollow, on a profile |
| `a` | Open the author's profile |
| `w` | Open in your browser |
| `D` | Delete your commit |
| `⇥` / `1` `2` `3` | Switch tabs |
| `g n` / `g b` / `g g` / `G` | Notifications / saved / top / bottom |
| `R` | Refresh (the log also counts new commits every minute, like the web) |
| `esc` | Back |
| `⌃C` / `Q` | Quit |

The mouse wheel scrolls. Click a commit to select it, and click it again to open it.

## Configuration

| Variable | |
|---|---|
| `DEVNULL_URL` | The server. Defaults to `https://dev-null.sh`. `http://localhost:8000` works for a local checkout. |
| `DEVNULL_CONFIG_DIR` | Where `config.json` and the device key live. Defaults to `~/.config/devnull` (`~/Library/Application Support/devnull` on macOS). |

Colors use the web's dark palette. On terminals without truecolor, they're mapped to the 256-color palette.

## Releasing

Bump `version` in `Cargo.toml` and push to `main`. The Release workflow sees a version with no tag yet, builds every platform, tags the commit and publishes the release with checksums and generated notes. Pushing a `v*` tag that matches `Cargo.toml` works too.
