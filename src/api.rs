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

    // Remotes -----------------------------------------------------------------------------------------------------

    /// What a Remotes answer failed to parse as says which one, so a change on the server is easy to place.
    fn parse<T: DeserializeOwned>(bytes: &[u8], what: &str) -> Result<T> {
        serde_json::from_slice(bytes).map_err(|e| anyhow::anyhow!("/dev/null changed its {what} answer ({e}). Update devnull."))
    }

    /// A page of the inbox. `tab` is open or stashed; `term` matches names and handles; `before` is the last
    /// conversation of the page before.
    pub async fn remotes(&self, tab: &str, term: &str, before: Option<&str>) -> Result<Inbox> {
        let mut path = format!("/api/v1/remotes?tab={}", encode(tab));
        if !term.is_empty() {
            path.push_str(&format!("&q={}", encode(term)));
        }
        if let Some(before) = before {
            path.push_str(&format!("&before={}", encode(before)));
        }
        let (_, bytes) = self.send(Method::GET, &path, None, None).await?;
        Self::parse(&bytes, "inbox")
    }

    /// The latest messages of a conversation, those after `after` when polling (with the reactions changed since
    /// `reactions_after`), or the ones before `before` when scrolling back.
    pub async fn remote_thread(
        &self,
        handle: &str,
        after: Option<i64>,
        before: Option<i64>,
        reactions_after: Option<i64>,
    ) -> Result<Thread> {
        let mut query = Vec::new();
        if let Some(after) = after {
            query.push(format!("after={after}"));
        }
        if let Some(before) = before {
            query.push(format!("before={before}"));
        }
        if let Some(version) = reactions_after {
            query.push(format!("reactions_after={version}"));
        }
        let mut path = format!("/api/v1/remotes/@{}/messages", encode(handle));
        if !query.is_empty() {
            path.push('?');
            path.push_str(&query.join("&"));
        }
        let (_, bytes) = self.send(Method::GET, &path, None, None).await?;
        Ok(Self::parse::<Data<Thread>>(&bytes, "conversation")?.data)
    }

    /// Pushes a message. `client_id` is a UUID the server dedupes retries by.
    pub async fn push_remote(&self, handle: &str, body: &str, client_id: &str, reply_to: Option<i64>) -> Result<Message> {
        let mut payload = json!({ "body": body, "client_id": client_id });
        if let Some(id) = reply_to {
            payload["reply_to_id"] = json!(id);
        }
        let (_, bytes) = self
            .send(
                Method::POST,
                &format!("/api/v1/remotes/@{}/messages", encode(handle)),
                Some(payload),
                None,
            )
            .await?;
        Ok(Self::parse::<Data<Message>>(&bytes, "message")?.data)
    }

    /// Pulls a conversation through a message. It only ever moves forward.
    pub async fn pull_remote(&self, handle: &str, through: i64) -> Result<()> {
        let body = json!({ "through": through });
        self.send(Method::PUT, &format!("/api/v1/remotes/@{}/read", encode(handle)), Some(body), None)
            .await
            .map(|_| ())
    }

    /// Stashes a conversation for the user only, or pops it back into Open.
    pub async fn stash_remote(&self, handle: &str, on: bool) -> Result<()> {
        let method = if on { Method::POST } else { Method::DELETE };
        self.write_empty(method, &format!("/api/v1/remotes/@{}/archive", encode(handle)))
            .await
    }

    /// Reacts with an emoji, or takes the reaction back. Answers with the message's reactions.
    pub async fn react(&self, message: i64, emoji: &str, on: bool) -> Result<Vec<Reaction>> {
        #[derive(serde::Deserialize)]
        struct Reactions {
            #[serde(default)]
            reactions: Vec<Reaction>,
        }
        let method = if on { Method::POST } else { Method::DELETE };
        let (_, bytes) = self
            .send(
                method,
                &format!("/api/v1/remotes/messages/{message}/reactions"),
                Some(json!({ "emoji": emoji })),
                None,
            )
            .await?;
        Ok(Self::parse::<Reactions>(&bytes, "reaction")?.reactions)
    }

    /// People to start a remote with, each with the reason they can't be messaged if there is one.
    pub async fn remote_candidates(&self, term: &str) -> Result<Vec<Candidate>> {
        #[derive(serde::Deserialize)]
        struct Candidates {
            #[serde(default)]
            candidates: Vec<Candidate>,
        }
        let (_, bytes) = self
            .send(Method::GET, &format!("/api/v1/remotes/candidates?q={}", encode(term)), None, None)
            .await?;
        Ok(Self::parse::<Candidates>(&bytes, "people")?.candidates)
    }

    /// Who can push to the user, and whether they share pulls.
    pub async fn set_remote_preferences(&self, from: &str, pull_receipts: bool) -> Result<()> {
        let body = json!({ "from": from, "pull_receipts": pull_receipts });
        self.send(Method::PUT, "/api/v1/me/remote-preferences", Some(body), None)
            .await
            .map(|_| ())
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

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;

    use super::*;

    /// Answers each request with the next canned body and sends back what it was asked: the request line and body.
    fn serve(answers: Vec<&'static str>) -> (String, std::thread::JoinHandle<Vec<(String, String)>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let handle = std::thread::spawn(move || {
            let mut seen = Vec::new();
            for answer in answers {
                let (mut stream, _) = listener.accept().unwrap();
                let mut buf = [0u8; 8192];
                let n = stream.read(&mut buf).unwrap();
                let request = String::from_utf8_lossy(&buf[..n]).to_string();
                let line = request.lines().next().unwrap_or_default().to_string();
                let body = request.split("\r\n\r\n").nth(1).unwrap_or_default().to_string();
                seen.push((line, body));
                let status = if answer.is_empty() { "204 No Content" } else { "200 OK" };
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{answer}",
                    answer.len()
                );
            }
            seen
        });
        (base, handle)
    }

    #[tokio::test]
    async fn remotes_calls_go_where_the_server_expects() {
        let (base, server) = serve(vec![
            r#"{"data":[],"has_more":false,"unread_count":3,"requests_count":0}"#,
            r#"{"data":{"partner":{"id":2,"handle":"petox03"},"messages":[],"access":"open"}}"#,
            r#"{"data":{"id":9,"user_id":1,"body":"hi"}}"#,
            "",
            "",
            r#"{"reactions":[{"user_id":1,"emoji":"👍"}]}"#,
            r#"{"candidates":[{"user":{"id":3,"handle":"x"},"reason":"No."}]}"#,
        ]);
        let api = Api::new(base, None);

        let inbox = api.remotes("stashed", "al", Some("abc")).await.unwrap();
        assert_eq!(inbox.unread_count, 3);
        let thread = api.remote_thread("petox03", Some(5), None, Some(2)).await.unwrap();
        assert_eq!(thread.access, "open");
        let message = api.push_remote("petox03", "hi", "id-1", Some(4)).await.unwrap();
        assert_eq!(message.id, 9);
        api.pull_remote("petox03", 9).await.unwrap();
        api.stash_remote("petox03", true).await.unwrap();
        assert_eq!(api.react(9, "👍", true).await.unwrap()[0].emoji, "👍");
        assert_eq!(api.remote_candidates("x y").await.unwrap()[0].reason.as_deref(), Some("No."));

        let seen = server.join().unwrap();
        let lines: Vec<&str> = seen.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(
            lines,
            [
                "GET /api/v1/remotes?tab=stashed&q=al&before=abc HTTP/1.1",
                "GET /api/v1/remotes/@petox03/messages?after=5&reactions_after=2 HTTP/1.1",
                "POST /api/v1/remotes/@petox03/messages HTTP/1.1",
                "PUT /api/v1/remotes/@petox03/read HTTP/1.1",
                "POST /api/v1/remotes/@petox03/archive HTTP/1.1",
                "POST /api/v1/remotes/messages/9/reactions HTTP/1.1",
                "GET /api/v1/remotes/candidates?q=x%20y HTTP/1.1",
            ]
        );
        let pushed: Value = serde_json::from_str(&seen[2].1).unwrap();
        assert_eq!(pushed, json!({ "body": "hi", "client_id": "id-1", "reply_to_id": 4 }));
        assert_eq!(serde_json::from_str::<Value>(&seen[3].1).unwrap(), json!({ "through": 9 }));
        assert_eq!(serde_json::from_str::<Value>(&seen[5].1).unwrap(), json!({ "emoji": "👍" }));
    }

    #[tokio::test]
    async fn an_answer_that_changed_says_which_one() {
        let (base, _server) = serve(vec![r#"{"data":{"messages":[]}}"#]);
        let error = Api::new(base, None).remote_thread("petox03", None, None, None).await.unwrap_err();
        assert!(error.to_string().contains("changed its conversation answer"), "{error}");
    }
}
