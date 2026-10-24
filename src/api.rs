use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use reqwest::{Method, StatusCode};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::auth::{API_NAMESPACE, DeviceKey, random_hex};
use crate::model::*;

#[derive(Debug, Clone)]
pub struct ApiError {
    pub code: Option<String>,
    pub message: String,
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ApiError {}

#[derive(Clone)]
pub struct Api {
    http: reqwest::Client,
    pub base: String,
    key: Option<Arc<DeviceKey>>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Log {
    Following,
    Latest,
    Trending,
}

impl Log {
    pub const ALL: [Log; 3] = [Log::Following, Log::Latest, Log::Trending];

    pub fn slug(self) -> &'static str {
        match self {
            Log::Following => "following",
            Log::Latest => "latest",
            Log::Trending => "trending",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Log::Following => "Following",
            Log::Latest => "Latest",
            Log::Trending => "Trending",
        }
    }
}

impl Api {
    pub fn new(base: String, key: Option<Arc<DeviceKey>>) -> Self {
        let http = reqwest::Client::builder()
            .user_agent(concat!("devnull-tui/", env!("CARGO_PKG_VERSION")))
            .timeout(Duration::from_secs(30))
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .expect("HTTP client");
        Self { http, base, key }
    }

    pub fn is_signed(&self) -> bool {
        self.key.is_some()
    }

    pub fn with_key(&self, key: Option<Arc<DeviceKey>>) -> Self {
        Self {
            http: self.http.clone(),
            base: self.base.clone(),
            key,
        }
    }

    /// Sends a request, signed when there is a device key: the timestamp, nonce, method, path with its query exactly
    /// as sent, and raw body, joined by newlines.
    async fn send(&self, method: Method, path: &str, body: Option<Value>, sign_with: Option<&DeviceKey>) -> Result<(StatusCode, Vec<u8>)> {
        let raw = body.map(|b| b.to_string()).unwrap_or_default();
        let mut request = self
            .http
            .request(method.clone(), format!("{}{}", self.base, path))
            .header("Accept", "application/json");

        if !raw.is_empty() {
            request = request.header("Content-Type", "application/json").body(raw.clone());
        }

        if let Some(key) = sign_with.or(self.key.as_deref()) {
            let timestamp = chrono::Utc::now().timestamp().to_string();
            let nonce = random_hex(16);
            let payload = [timestamp.as_str(), nonce.as_str(), method.as_str(), path, raw.as_str()].join("\n");
            request = request
                .header("X-Devnull-Timestamp", timestamp)
                .header("X-Devnull-Nonce", nonce)
                .header("X-SSH-Signature", key.sign(API_NAMESPACE, payload.as_bytes())?);
        }

        let response = request.send().await.map_err(|e| ApiError {
            code: None,
            message: if e.is_timeout() {
                "/dev/null took too long to answer.".into()
            } else {
                format!("Could not reach {}.", self.base)
            },
        })?;
        let status = response.status();
        let bytes = response.bytes().await?.to_vec();

        if !status.is_success() {
            let body: Option<ApiErrorBody> = serde_json::from_slice(&bytes).ok();
            let message = body
                .as_ref()
                .and_then(|b| b.message.clone())
                .filter(|m| !m.is_empty())
                .unwrap_or_else(|| match status.as_u16() {
                    404 => "Not found.".into(),
                    429 => "Slow down: too many requests. Try again in a minute.".into(),
                    s => format!("/dev/null answered HTTP {s}."),
                });
            return Err(ApiError {
                code: body.and_then(|b| b.code),
                message,
            }
            .into());
        }

        Ok((status, bytes))
    }

    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let (_, bytes) = self.send(Method::GET, path, None, None).await?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    async fn write<T: DeserializeOwned>(&self, method: Method, path: &str, body: Option<Value>) -> Result<T> {
        let (_, bytes) = self.send(method, path, body, None).await?;
        Ok(serde_json::from_slice(&bytes)?)
    }

    async fn write_empty(&self, method: Method, path: &str) -> Result<()> {
        self.send(method, path, None, None).await.map(|_| ())
    }

    fn cursor(path: String, cursor: Option<&str>) -> String {
        match cursor {
            Some(c) => format!("{path}{}cursor={}", if path.contains('?') { '&' } else { '?' }, encode(c)),
            None => path,
        }
    }

    pub async fn me(&self) -> Result<CurrentUser> {
        Ok(self.get::<Data<CurrentUser>>("/api/v1/me").await?.data)
    }

    pub async fn log(&self, log: Log, cursor: Option<&str>) -> Result<Page<Commit>> {
        self.get(&Self::cursor(format!("/api/v1/log/{}", log.slug()), cursor)).await
    }

    pub async fn new_commits_since(&self, log: Log, since: &str) -> Result<u64> {
        let page: Page<Commit> = self.get(&format!("/api/v1/log/{}?since={}", log.slug(), encode(since))).await?;
        Ok(page.new_commits_count.unwrap_or(0))
    }

    pub async fn user(&self, username: &str) -> Result<Profile> {
        self.get(&format!("/api/v1/users/{}", encode(username))).await
    }

    pub async fn user_commits(&self, username: &str, replies: bool, cursor: Option<&str>) -> Result<Page<Commit>> {
        let path = format!(
            "/api/v1/users/{}/commits{}",
            encode(username),
            if replies { "?replies=1" } else { "" }
        );
        self.get(&Self::cursor(path, cursor)).await
    }

    pub async fn following(&self, username: &str, cursor: Option<&str>) -> Result<Page<User>> {
        self.get(&Self::cursor(format!("/api/v1/users/{}/following", encode(username)), cursor))
            .await
    }

    pub async fn followers(&self, username: &str, cursor: Option<&str>) -> Result<Page<User>> {
        self.get(&Self::cursor(format!("/api/v1/users/{}/followers", encode(username)), cursor))
            .await
    }

    pub async fn mute(&self, username: &str, on: bool) -> Result<()> {
        let method = if on { Method::POST } else { Method::DELETE };
        self.send(method, &format!("/api/v1/users/{}/mute", encode(username)), None, None)
            .await
            .map(|_| ())
    }

    pub async fn block(&self, username: &str, on: bool) -> Result<()> {
        let method = if on { Method::POST } else { Method::DELETE };
        self.send(method, &format!("/api/v1/users/{}/block", encode(username)), None, None)
            .await
            .map(|_| ())
    }

    /// Reports a commit to the moderators, privately. `reason` is spam, harassment, unsafe or other.
    pub async fn report(&self, id: &str, reason: &str) -> Result<()> {
        let body = json!({ "reason": reason });
        self.send(Method::POST, &format!("/api/v1/commits/{}/reports", encode(id)), Some(body), None)
            .await
            .map(|_| ())
    }

    pub async fn commit(&self, id: &str) -> Result<CommitPage> {
        self.get(&format!("/api/v1/commits/{}", encode(id))).await
    }

    pub async fn replies(&self, id: &str, cursor: Option<&str>) -> Result<Page<Commit>> {
        self.get(&Self::cursor(format!("/api/v1/commits/{}/replies", encode(id)), cursor))
            .await
    }

    /// The commits the user starred, last starred first. Stars replaced saving.
    pub async fn starred(&self, cursor: Option<&str>) -> Result<Page<Commit>> {
        self.get(&Self::cursor("/api/v1/starred".into(), cursor)).await
    }

    /// Listing notifications marks them all as read, whatever `kind` filters them to.
    pub async fn notifications(&self, kind: &str, cursor: Option<&str>) -> Result<Page<Notification>> {
        self.get(&Self::cursor(format!("/api/v1/notifications?type={}", encode(kind)), cursor))
            .await
    }

    pub async fn create_commit(&self, body: &str, parent_id: Option<&str>, quoted_commit_id: Option<&str>) -> Result<Commit> {
        let mut payload = json!({ "body": body });
        if let Some(id) = parent_id {
            payload["parent_id"] = json!(id);
        }
        if let Some(id) = quoted_commit_id {
            payload["quoted_commit_id"] = json!(id);
        }
        Ok(self
            .write::<Data<Commit>>(Method::POST, "/api/v1/commits", Some(payload))
            .await?
            .data)
    }

    pub async fn delete_commit(&self, id: &str) -> Result<()> {
        self.write_empty(Method::DELETE, &format!("/api/v1/commits/{}", encode(id))).await
    }

    pub async fn star(&self, id: &str, on: bool) -> Result<Commit> {
        let method = if on { Method::POST } else { Method::DELETE };
        Ok(self
            .write::<Data<Commit>>(method, &format!("/api/v1/commits/{}/star", encode(id)), None)
            .await?
            .data)
    }

    pub async fn pastel(&self, id: &str) -> Result<Commit> {
        Ok(self
            .write::<Data<Commit>>(Method::POST, &format!("/api/v1/commits/{}/pasteis", encode(id)), None)
            .await?
            .data)
    }

    pub async fn follow(&self, username: &str, on: bool) -> Result<()> {
        let method = if on { Method::POST } else { Method::DELETE };
        self.write_empty(method, &format!("/api/v1/users/{}/follow", encode(username)))
            .await
    }

    /// A lane from 0 to 5, a custom `#rrggbb` color, or neither for Automatic.
    pub async fn set_lane(&self, lane: Option<u8>, color: Option<&str>) -> Result<CurrentUser> {
        let body = match color {
            Some(color) => json!({ "lane": null, "color": color }),
            None => json!({ "lane": lane }),
        };
        Ok(self
            .write::<Data<CurrentUser>>(Method::PUT, "/api/v1/me/lane", Some(body))
            .await?
            .data)
    }

    pub async fn set_pasteis_mode(&self, enabled: bool) -> Result<CurrentUser> {
        let body = json!({ "enabled": enabled });
        Ok(self
            .write::<Data<CurrentUser>>(Method::PUT, "/api/v1/me/pasteis-mode", Some(body))
            .await?
            .data)
    }

    /// Mirrors the profile from GitHub again, as Sync from GitHub does on the web. Once a minute at most.
    pub async fn sync_github(&self) -> Result<CurrentUser> {
        Ok(self.write::<Data<CurrentUser>>(Method::PUT, "/api/v1/me/github", None).await?.data)
    }

    /// Redeems the code from the authorize page, signed with the key that isn't approved yet.
    pub async fn redeem(&self, key: &DeviceKey, code: &str, verifier: &str) -> Result<CurrentUser> {
        let body = json!({ "code": code, "code_verifier": verifier });
        let (_, bytes) = self.send(Method::POST, "/api/v1/devices", Some(body), Some(key)).await?;
        Ok(serde_json::from_slice::<Data<CurrentUser>>(&bytes)?.data)
    }

    pub fn http(&self) -> &reqwest::Client {
        &self.http
    }
}

pub fn encode(segment: &str) -> String {
    url::form_urlencoded::byte_serialize(segment.as_bytes())
        .collect::<String>()
        .replace('+', "%20")
}

pub fn api_error(err: &anyhow::Error) -> Option<&ApiError> {
    err.downcast_ref::<ApiError>()
}
