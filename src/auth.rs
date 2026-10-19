//! Signing in as a device, the way the mobile app does: an ed25519 key kept on this machine signs every request
//! (SSHSIG, namespace "devnull-api"), and the user approves it once on the web with PKCE.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::RngCore;
use rand::rngs::OsRng;
use sha2::{Digest, Sha256};
use ssh_key::{Algorithm, HashAlg, LineEnding, PrivateKey};

pub const API_NAMESPACE: &str = "devnull-api";
pub const REDIRECT_URI: &str = "devnull://auth/callback";

pub struct DeviceKey {
    key: PrivateKey,
}

impl DeviceKey {
    pub fn load(path: &Path) -> Option<Self> {
        let pem = fs::read_to_string(path).ok()?;
        PrivateKey::from_openssh(pem).ok().map(|key| Self { key })
    }

    pub fn load_or_create(path: &Path) -> Result<Self> {
        if let Some(key) = Self::load(path) {
            return Ok(key);
        }

        let key = PrivateKey::random(&mut OsRng, Algorithm::Ed25519).map_err(|e| anyhow!("Could not make a key: {e}"))?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let pem = key.to_openssh(LineEnding::LF).map_err(|e| anyhow!("{e}"))?;
        write_private(path, pem.as_bytes())?;
        fs::write(path.with_extension("pub"), key.public_key().to_openssh().unwrap_or_default() + "\n")?;

        Ok(Self { key })
    }

    pub fn fingerprint(&self) -> String {
        self.key.public_key().fingerprint(HashAlg::Sha256).to_string()
    }

    /// `ssh-keygen -Y sign -n <namespace>`, as base64 on one line.
    pub fn sign(&self, namespace: &str, message: &[u8]) -> Result<String> {
        let signature = self.key.sign(namespace, HashAlg::Sha512, message).map_err(|e| anyhow!("Could not sign: {e}"))?;
        let pem = signature.to_pem(LineEnding::LF).map_err(|e| anyhow!("{e}"))?;
        Ok(pem.lines().filter(|line| !line.starts_with("-----")).collect())
    }
}

#[cfg(unix)]
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path).context("Could not save the device key")?;
    file.write_all(bytes)?;
    Ok(())
}

#[cfg(not(unix))]
fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    fs::write(path, bytes).context("Could not save the device key")
}

pub fn random_hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    OsRng.fill_bytes(&mut buf);
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

/// One sign-in attempt: what goes to the authorize page, and what the redeemed code has to match.
pub struct Authorization {
    pub verifier: String,
    pub state: String,
    pub url: String,
}

impl Authorization {
    pub fn new(base_url: &str, fingerprint: &str) -> Self {
        let mut bytes = [0u8; 32];
        OsRng.fill_bytes(&mut bytes);
        let verifier = URL_SAFE_NO_PAD.encode(bytes);
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let state = random_hex(16);

        let host = hostname();
        let device_name = if host.is_empty() { "devnull TUI".to_string() } else { format!("devnull TUI on {host}") };

        let mut url = url::Url::parse(&format!("{base_url}/mobile/authorize")).expect("valid base URL");
        url.query_pairs_mut()
            .append_pair("fingerprint", fingerprint)
            .append_pair("device_name", &device_name)
            .append_pair("code_challenge", &challenge)
            .append_pair("code_challenge_method", "S256")
            .append_pair("state", &state)
            .append_pair("redirect_uri", REDIRECT_URI);

        Self { verifier, state, url: url.into() }
    }

    /// Takes the `devnull://auth/callback?code=…&state=…` link the browser was sent to, or just the code.
    pub fn code_from(&self, pasted: &str) -> Result<String> {
        let pasted = pasted.trim();
        if pasted.is_empty() {
            bail!("Paste the devnull:// link your browser was sent to.");
        }
        if !pasted.contains("://") && !pasted.contains('?') {
            return Ok(pasted.to_string());
        }

        let query = pasted.split_once('?').map(|(_, q)| q).unwrap_or("");
        let pairs: Vec<(String, String)> = url::form_urlencoded::parse(query.as_bytes()).into_owned().collect();
        let get = |name: &str| pairs.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone());

        if let Some(error) = get("error") {
            bail!(if error == "access_denied" { "You cancelled the sign-in.".to_string() } else { format!("Sign-in failed: {error}") });
        }
        if get("state").as_deref() != Some(self.state.as_str()) {
            bail!("That link is from another sign-in. Start again.");
        }
        get("code").ok_or_else(|| anyhow!("That link has no code in it."))
    }
}

fn hostname() -> String {
    std::env::var("HOSTNAME")
        .ok()
        .or_else(|| fs::read_to_string("/etc/hostname").ok())
        .or_else(|| {
            std::process::Command::new("hostname").output().ok().and_then(|o| String::from_utf8(o.stdout).ok())
        })
        .map(|h| h.trim().trim_end_matches(".local").to_string())
        .unwrap_or_default()
        .chars()
        .take(60)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_challenge_is_the_hash_of_the_verifier() {
        let auth = Authorization::new("https://dev-null.sh", "SHA256:abc");
        let url = url::Url::parse(&auth.url).unwrap();
        let challenge = url.query_pairs().find(|(k, _)| k == "code_challenge").unwrap().1.to_string();
        assert_eq!(challenge, URL_SAFE_NO_PAD.encode(Sha256::digest(auth.verifier.as_bytes())));
        assert_eq!(challenge.len(), 43);
        assert!((43..=128).contains(&auth.verifier.len()));
    }

    #[test]
    fn it_takes_the_code_from_the_callback_link() {
        let auth = Authorization::new("https://dev-null.sh", "SHA256:abc");
        let link = format!("devnull://auth/callback?code=xyz&state={}", auth.state);
        assert_eq!(auth.code_from(&link).unwrap(), "xyz");
        assert_eq!(auth.code_from("  just-the-code ").unwrap(), "just-the-code");
        assert!(auth.code_from("devnull://auth/callback?code=xyz&state=someone-else").is_err());
        let denied = format!("devnull://auth/callback?error=access_denied&state={}", auth.state);
        assert!(auth.code_from(&denied).unwrap_err().to_string().contains("cancelled"));
    }

    #[test]
    fn signatures_are_one_line_of_base64() {
        let dir = std::env::temp_dir().join(format!("devnull-test-{}", random_hex(4)));
        let key = DeviceKey::load_or_create(&dir.join("key")).unwrap();
        let signature = key.sign(API_NAMESPACE, b"hello").unwrap();
        assert!(!signature.contains('\n') && !signature.contains("-----"));
        assert!(key.fingerprint().starts_with("SHA256:"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
