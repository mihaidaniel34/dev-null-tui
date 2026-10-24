//! The JSON the /dev/null API answers with. Everything a signed request adds on top of the public view is optional,
//! so the same types read both.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer};
use serde_json::Value;

#[derive(Debug, Clone, Deserialize)]
pub struct Data<T> {
    pub data: T,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Meta {
    #[serde(default)]
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Page<T> {
    pub data: Vec<T>,
    #[serde(default)]
    pub meta: Meta,
    #[serde(default)]
    pub new_commits_count: Option<u64>,
    #[serde(default)]
    pub suggestions: Vec<Commit>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct User {
    #[serde(deserialize_with = "string_or_number")]
    pub id: String,
    pub username: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub avatar_url: Option<String>,
    #[serde(default)]
    pub bio: Option<String>,
    #[serde(default)]
    pub company: Option<String>,
    #[serde(default)]
    pub location: Option<String>,
    #[serde(default)]
    pub website: Option<String>,
    #[serde(default)]
    pub commits_count: Option<u64>,
    #[serde(default)]
    pub followers_count: Option<u64>,
    #[serde(default)]
    pub following_count: Option<u64>,
    /// A lane from 0 to 5, or a custom `#rrggbb`. Only signed requests get it.
    #[serde(default)]
    pub lane: Option<Value>,
    /// In a signed list of people: whether the user follows each one, and whether each one follows them.
    #[serde(default)]
    pub is_followed: Option<bool>,
    #[serde(default)]
    pub follows_you: Option<bool>,
}

impl User {
    pub fn display_name(&self) -> &str {
        self.name.as_deref().filter(|n| !n.trim().is_empty()).unwrap_or(&self.username)
    }
}

/// `GET /api/v1/users/{username}`, with what a signed request adds next to `data`.
#[derive(Debug, Clone, Deserialize)]
pub struct Profile {
    pub data: User,
    #[serde(default)]
    pub is_following: Option<bool>,
    #[serde(default)]
    pub follows_you: Option<bool>,
    #[serde(default)]
    pub pinned_commit: Option<Commit>,
    #[serde(default)]
    pub is_muting: Option<bool>,
    #[serde(default)]
    pub activity: Option<Activity>,
    #[serde(default)]
    pub streaks: Option<Streaks>,
    #[serde(default)]
    pub achievements: Option<Achievements>,
}

/// Commits per hour (scale "day") or per day (week, month, year), oldest first. The year starts on a Sunday.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct Activity {
    #[serde(default)]
    pub scale: String,
    #[serde(default)]
    pub buckets: Vec<Bucket>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Bucket {
    #[serde(default)]
    pub date: String,
    #[serde(default)]
    pub count: u64,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Streaks {
    #[serde(default)]
    pub current: u64,
    #[serde(default)]
    pub longest: u64,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Achievements {
    #[serde(default)]
    pub level: u64,
    #[serde(default)]
    pub rank: String,
    #[serde(default)]
    pub xp: i64,
    #[serde(default)]
    pub next_level_xp: Option<i64>,
    #[serde(default)]
    pub level_progress: u64,
    #[serde(default)]
    pub badges: Vec<Badge>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Badge {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub unlocked: bool,
    #[serde(default)]
    pub progress: u64,
    #[serde(default)]
    pub target: u64,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct CurrentUser {
    #[serde(flatten)]
    pub user: User,
    #[serde(default)]
    pub pasteis_mode: bool,
    #[serde(default)]
    pub pasteis_remaining: Option<u64>,
    /// A lane from 0 to 5, a custom `#rrggbb`, or null for Automatic.
    #[serde(default)]
    pub preferred_lane: Option<Value>,
    #[serde(default)]
    pub automatic_lane: Option<u8>,
    #[serde(default)]
    pub github_synced_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct LinkPreview {
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Image {
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub width: Option<u64>,
    #[serde(default)]
    pub height: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct PollOption {
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub voted: bool,
    #[serde(default)]
    pub percentage: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Poll {
    #[serde(default)]
    pub is_closed: bool,
    #[serde(default)]
    pub total_votes: Option<u64>,
    #[serde(default)]
    pub options: Vec<PollOption>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ReplyPreview {
    #[serde(default)]
    pub data: Vec<Commit>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Commit {
    pub id: String,
    #[serde(default)]
    pub short_hash: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub html_url: Option<String>,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub stars_count: Option<u64>,
    #[serde(default)]
    pub replies_count: Option<u64>,
    #[serde(default)]
    pub link_preview: Option<LinkPreview>,
    #[serde(default)]
    pub images: Option<Vec<Image>>,
    #[serde(default)]
    pub author: Option<User>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub is_pinned: Option<bool>,
    #[serde(default)]
    pub starred: Option<bool>,
    #[serde(default)]
    pub pasteis_count: Option<u64>,
    #[serde(default)]
    pub deleted_at: Option<String>,
    #[serde(default)]
    pub quoted_commit_id: Option<String>,
    #[serde(default)]
    pub quoted_commit: Option<Box<Commit>>,
    #[serde(default)]
    pub poll: Option<Poll>,
    #[serde(default)]
    pub reply_preview: Option<ReplyPreview>,
    /// The body split into prose, inline code and code blocks the server highlighted. Signed requests only.
    #[serde(default)]
    pub body_segments: Option<Vec<Segment>>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Segment {
    #[serde(rename = "type", default)]
    pub kind: String,
    /// A block's code, as HTML spans with tempest/highlight's `hl-*` classes.
    #[serde(default)]
    pub html: Option<String>,
}

impl Commit {
    /// Each code block's highlighted HTML, in order, when the server sent them.
    pub fn highlighted_blocks(&self) -> Vec<String> {
        self.body_segments
            .iter()
            .flatten()
            .filter(|s| s.kind == "block")
            .map(|s| s.html.clone().unwrap_or_default())
            .collect()
    }
}

impl Commit {
    pub fn is_tombstone(&self) -> bool {
        self.deleted_at.is_some() || self.author.is_none()
    }

    pub fn hash(&self) -> String {
        self.short_hash.clone().unwrap_or_else(|| {
            let compact: String = self.id.chars().filter(|c| *c != '-').collect();
            compact.chars().rev().take(7).collect::<Vec<_>>().into_iter().rev().collect()
        })
    }

    /// Takes what a star, save or pastel answered with, keeping what that answer leaves out.
    pub fn merge(&mut self, fresh: Commit) {
        let reply_preview = self.reply_preview.take();
        *self = Commit {
            reply_preview: fresh.reply_preview.clone().or(reply_preview),
            ..fresh
        };
    }
}

/// `GET /api/v1/commits/{id}`: a signed request gets the commits it replies to as well.
#[derive(Debug, Clone, Deserialize)]
pub struct CommitPage {
    pub data: Commit,
    #[serde(default)]
    pub ancestors: Vec<Commit>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Notification {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub actor: Option<User>,
    #[serde(default)]
    pub actors_count: Option<u64>,
    #[serde(default)]
    pub unread_count: Option<u64>,
    #[serde(default)]
    pub commit: Option<Commit>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ApiErrorBody {
    #[serde(default)]
    pub message: Option<String>,
    #[serde(default)]
    pub code: Option<String>,
}

fn string_or_number<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
    Ok(match Value::deserialize(deserializer)? {
        Value::String(s) => s,
        other => other.to_string(),
    })
}
