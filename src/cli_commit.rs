//! Committing without signing in, exactly as the `curl -fsSL https://dev-null.sh/commit | bash` script does: find a
//! local SSH key that is on the user's GitHub account, sign the commit with `ssh-keygen -Y sign -n devnull-commit`
//! (so the agent, passphrases and hardware keys all work), and send it to /api/cli/commits.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, anyhow, bail};

use crate::api::Api;
use crate::auth::random_hex;

pub const NAMESPACE: &str = "devnull-commit";

pub struct SigningKey {
    /// What to pass to `ssh-keygen -f`: the public key when the agent holds it, otherwise the private key file.
    path: PathBuf,
    /// The private key file may ask for its passphrase on the terminal, so the TUI steps aside while it signs.
    pub needs_terminal: bool,
    pub label: String,
    tmp_dir: PathBuf,
}

impl Drop for SigningKey {
    fn drop(&mut self) {
        for file in ["key.pub", "commit", "commit.sig"] {
            let _ = fs::remove_file(self.tmp_dir.join(file));
        }
        let _ = fs::remove_dir(&self.tmp_dir);
    }
}

fn key_id(line: &str) -> Option<String> {
    let mut parts = line.split_whitespace();
    let (kind, blob) = (parts.next()?, parts.next()?);
    matches!(kind, "ssh-ed25519" | "ssh-rsa").then(|| format!("{kind} {blob}"))
}

fn agent_keys() -> Vec<String> {
    Command::new("ssh-add")
        .arg("-L")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().filter_map(key_id).collect())
        .unwrap_or_default()
}

/// Public keys in ~/.ssh that have their private key next to them.
fn file_keys() -> Vec<(String, PathBuf)> {
    let Some(ssh) = dirs::home_dir().map(|h| h.join(".ssh")) else { return vec![] };
    let Ok(entries) = fs::read_dir(&ssh) else { return vec![] };
    let mut keys: Vec<(String, PathBuf)> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "pub") && p.with_extension("").is_file())
        .filter_map(|p| {
            let first = fs::read_to_string(&p).ok()?.lines().next()?.to_string();
            Some((key_id(&first)?, p.with_extension("")))
        })
        .collect();
    keys.sort_by(|a, b| a.1.cmp(&b.1));
    keys
}

/// Asks /dev/null which local key is on a user's GitHub account, and only ever signs with one of our own.
pub async fn find_key(api: &Api) -> Result<SigningKey> {
    if Command::new("ssh-keygen").arg("-?").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).status().is_err() {
        bail!("ssh-keygen is required to commit without signing in.");
    }

    let agent = agent_keys();
    let files = file_keys();
    let mut candidates: Vec<String> = agent.clone();
    for (id, _) in &files {
        if !candidates.contains(id) {
            candidates.push(id.clone());
        }
    }
    if candidates.is_empty() {
        bail!("No Ed25519 or RSA SSH key found in your SSH agent or in ~/.ssh.");
    }

    let response = api
        .http()
        .post(format!("{}/api/cli/key-lookups", api.base))
        .header("Content-Type", "text/plain; charset=utf-8")
        .header("Accept", "application/json")
        .body(candidates.join("\n") + "\n")
        .send()
        .await
        .map_err(|_| anyhow!("Could not reach {}.", api.base))?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!("{}", plain_message(&text, status.as_u16()));
    }

    let key = text.lines().next().and_then(key_id).filter(|k| candidates.contains(k)).ok_or_else(|| anyhow!("Unexpected answer from {}.", api.base))?;

    let tmp_dir = std::env::temp_dir().join(format!("devnull.{}", random_hex(5)));
    create_private_dir(&tmp_dir)?;

    if agent.contains(&key) {
        let path = tmp_dir.join("key.pub");
        fs::write(&path, format!("{key}\n"))?;
        return Ok(SigningKey { path, needs_terminal: false, label: "your SSH agent".into(), tmp_dir });
    }

    let (_, path) = files.into_iter().find(|(id, _)| *id == key).ok_or_else(|| anyhow!("Could not find the private key of {key}."))?;
    let label = path.display().to_string().replace(&dirs::home_dir().unwrap_or_default().display().to_string(), "~");
    Ok(SigningKey { path, needs_terminal: true, label, tmp_dir })
}

#[cfg(unix)]
fn create_private_dir(path: &PathBuf) -> Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    fs::DirBuilder::new().mode(0o700).create(path).context("Could not create a private temporary directory.")
}

#[cfg(not(unix))]
fn create_private_dir(path: &PathBuf) -> Result<()> {
    fs::create_dir(path).context("Could not create a private temporary directory.")
}

/// "<unix timestamp>\n<nonce>\n<commit to reply to, or empty>\n<body>"
pub fn payload(reply_to: Option<&str>, body: &str) -> String {
    format!("{}\n{}\n{}\n{}", chrono::Utc::now().timestamp(), random_hex(16), reply_to.unwrap_or(""), body)
}

/// Runs `ssh-keygen -Y sign`. Blocks, and may prompt for a passphrase when `needs_terminal`: ssh-keygen asks on
/// /dev/tty, so its chatter on stderr can still be kept for when it fails.
pub fn sign(key: &SigningKey, payload: &str) -> Result<String> {
    let file = key.tmp_dir.join("commit");
    let _ = fs::remove_file(key.tmp_dir.join("commit.sig"));
    fs::write(&file, payload)?;

    let output = Command::new("ssh-keygen")
        .args(["-Y", "sign", "-n", NAMESPACE, "-f"])
        .arg(&key.path)
        .arg(&file)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .context("Could not run ssh-keygen.")?;

    if !output.status.success() {
        let reason = String::from_utf8_lossy(&output.stderr).lines().last().unwrap_or("").to_string();
        bail!("Could not sign your commit with {}. {}", key.label, reason);
    }

    let pem = fs::read_to_string(key.tmp_dir.join("commit.sig")).context("ssh-keygen produced no signature.")?;
    let signature: String = pem.lines().filter(|l| !l.starts_with("-----")).collect();
    if signature.is_empty() || !signature.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=')) {
        bail!("ssh-keygen produced an unexpected signature.");
    }
    Ok(signature)
}

/// Sends the signed commit, and answers with what /dev/null said ("Committed: <link>").
pub async fn send(api: &Api, payload: String, signature: String) -> Result<String> {
    let response = api
        .http()
        .post(format!("{}/api/cli/commits", api.base))
        .header("Content-Type", "text/plain; charset=utf-8")
        .header("Accept", "application/json")
        .header("X-SSH-Signature", signature)
        .body(payload)
        .send()
        .await
        .map_err(|_| anyhow!("Could not reach {}.", api.base))?;
    let status = response.status();
    let text = response.text().await.unwrap_or_default();
    if !status.is_success() {
        bail!("{}", plain_message(&text, status.as_u16()));
    }
    Ok(sanitize(text.trim()))
}

/// The CLI endpoints answer in plain text, or JSON for validation errors.
fn plain_message(text: &str, status: u16) -> String {
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(text) {
        if let Some(message) = value.get("message").and_then(|m| m.as_str()) {
            return sanitize(message);
        }
    }
    let text = sanitize(text.trim());
    if text.is_empty() || text.starts_with('<') { format!("/dev/null answered HTTP {status}.") } else { text }
}

fn sanitize(text: &str) -> String {
    text.chars().filter(|c| !c.is_control() || *c == '\n').collect()
}
