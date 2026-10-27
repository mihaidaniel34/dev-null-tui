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
    #[serde(alias = "handle")]
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
    /// The user's Remotes settings, or null while Remotes are off.
    #[serde(default)]
    pub remotes: Option<RemotesSettings>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct RemotesSettings {
    /// Who can push to the user: following or everyone.
    #[serde(default)]
    pub from: String,
    #[serde(default)]
    pub pull_receipts: bool,
    #[serde(default)]
    pub unread_count: u64,
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

// Remotes ---------------------------------------------------------------------------------------------------------
//
// Every field is optional or defaulted, so a field the server adds is ignored and one it drops leaves a gap rather
// than a failure. Only what a screen can't do without (a message's id, a conversation's id) is required.

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Reaction {
    #[serde(default, deserialize_with = "string_or_number")]
    pub user_id: String,
    #[serde(default)]
    pub emoji: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ReplyTo {
    #[serde(default, deserialize_with = "string_or_number")]
    pub user_id: String,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub images_count: u64,
    #[serde(default)]
    pub has_voice: bool,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct Message {
    #[serde(deserialize_with = "number_or_string")]
    pub id: i64,
    #[serde(default, deserialize_with = "string_or_number")]
    pub user_id: String,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub body_segments: Option<Vec<Segment>>,
    #[serde(default)]
    pub reply_to: Option<ReplyTo>,
    #[serde(default)]
    pub link_preview: Option<LinkPreview>,
    #[serde(default)]
    pub images: Option<Vec<Image>>,
    /// A voice note. The terminal can't play it, so it only says there is one.
    #[serde(default)]
    pub voice: Option<Value>,
    #[serde(default, deserialize_with = "or_default")]
    pub reactions: Vec<Reaction>,
    /// Set on an org's events, such as someone joining. Direct conversations have none.
    #[serde(default)]
    pub event: Option<String>,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
}

impl Message {
    pub fn highlighted_blocks(&self) -> Vec<String> {
        self.body_segments
            .iter()
            .flatten()
            .filter(|s| s.kind == "block")
            .map(|s| s.html.clone().unwrap_or_default())
            .collect()
    }
}

/// A row of the inbox. An org has no `partner`; the terminal shows it but doesn't open it.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct Conversation {
    #[serde(default)]
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub partner: Option<User>,
    #[serde(default)]
    pub last_message: Option<Message>,
    #[serde(default)]
    pub last_message_at: Option<DateTime<Utc>>,
    /// How many messages from others haven't been pulled yet.
    #[serde(default)]
    pub ahead: u64,
    #[serde(default)]
    pub is_archived: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Inbox {
    pub data: Vec<Conversation>,
    #[serde(default)]
    pub has_more: bool,
    #[serde(default)]
    pub unread_count: u64,
    #[serde(default)]
    pub requests_count: u64,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ReactionChange {
    #[serde(default, deserialize_with = "number_or_string")]
    pub id: i64,
    #[serde(default)]
    pub reactions: Vec<Reaction>,
}

/// `GET /api/v1/remotes/@handle/messages`, inside `data`.
#[derive(Debug, Clone, Deserialize)]
pub struct Thread {
    pub partner: User,
    #[serde(default, deserialize_with = "or_default")]
    pub messages: Vec<Message>,
    #[serde(default)]
    pub has_older: bool,
    #[serde(default)]
    pub reactions_version: i64,
    #[serde(default, deserialize_with = "or_default")]
    pub reaction_changes: Vec<ReactionChange>,
    #[serde(default)]
    pub read_through_id: i64,
    #[serde(default)]
    pub partner_read_through_id: i64,
    #[serde(default)]
    pub partner_read_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub partner_typing: bool,
    #[serde(default)]
    pub is_archived: bool,
    /// What the composer can do: open, request, waiting, restricted or closed.
    #[serde(default)]
    pub access: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Candidate {
    pub user: User,
    /// Why they can't be messaged, if they can't.
    #[serde(default)]
    pub reason: Option<String>,
}

fn number_or_string<'de, D: Deserializer<'de>>(deserializer: D) -> Result<i64, D::Error> {
    Ok(match Value::deserialize(deserializer)? {
        Value::Number(n) => n.as_i64().unwrap_or_default(),
        Value::String(s) => s.parse().unwrap_or_default(),
        _ => 0,
    })
}

/// A field the server may send as null where a list or a number is expected.
fn or_default<'de, D: Deserializer<'de>, T: Deserialize<'de> + Default>(deserializer: D) -> Result<T, D::Error> {
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The shapes the server's own tests assert for the Remotes endpoints (tests/Feature/Controllers/Api).
    #[test]
    fn reads_a_thread() {
        let answer = json!({ "data": {
            "partner": { "id": 7, "handle": "nunomaduro", "name": "Nuno", "lane": 2, "something_new": true },
            "messages": [
                { "id": 11, "user_id": 7, "client_id": "c", "body": "did @octocat see `the PR`?", "body_segments": [],
                  "mentions": [], "reply_to": null, "link_preview": { "url": "https://laravel.com", "is_pending": true },
                  "event": null, "created_at": "2026-10-27T17:00:00Z" },
                { "id": 12, "user_id": 3, "body": "", "reply_to_id": 11,
                  "reply_to": { "id": 11, "user_id": 7, "body": "did", "images_count": 0, "has_voice": false },
                  "images": [{ "id": "i", "width": 10, "height": 5, "description": "The diff", "url": "https://x/y" }],
                  "reactions": [{ "user_id": 7, "emoji": "\u{2764}\u{fe0f}" }],
                  "created_at": "2026-10-27T17:01:00Z" }
            ],
            "has_older": false, "reactions_version": 1, "reaction_changes": [],
            "read_through_id": 12, "partner_read_through_id": 0, "partner_read_at": null,
            "partner_typing": false, "is_archived": false, "pinned_at": null, "muted_until_at": null, "access": "open"
        }});
        let thread = serde_json::from_value::<Data<Thread>>(answer).unwrap().data;
        assert_eq!(thread.partner.username, "nunomaduro");
        assert_eq!(thread.partner.id, "7");
        assert_eq!(thread.messages[1].reply_to.as_ref().unwrap().user_id, "7");
        assert_eq!(thread.messages[1].reactions[0].user_id, "7");
        assert_eq!(thread.messages[1].images.as_ref().unwrap()[0].url.as_deref(), Some("https://x/y"));
        assert_eq!(thread.access, "open");
    }

    #[test]
    fn reads_an_inbox_with_an_org_and_nulls() {
        let answer = json!({
            "data": [
                { "id": "a", "kind": "direct", "name": null, "partner": { "id": 1, "handle": "petox03" },
                  "last_message": { "id": 5, "user_id": 1, "body": "hi", "reactions": null },
                  "last_message_at": "2026-10-27T17:00:00Z", "ahead": 2, "is_archived": false },
                { "id": "b", "kind": "org", "name": "Team", "partner": null, "members": [], "last_message": null }
            ],
            "has_more": false, "unread_count": 2, "requests_count": 0
        });
        let inbox: Inbox = serde_json::from_value(answer).unwrap();
        assert_eq!(inbox.data[0].ahead, 2);
        assert!(inbox.data[1].partner.is_none());
        assert_eq!(inbox.unread_count, 2);
    }

    #[test]
    fn reads_the_remotes_settings_of_the_current_user() {
        let on: CurrentUser = serde_json::from_value(json!({
            "id": 1, "handle": "me", "remotes": { "from": "following", "pull_receipts": true, "requests": false, "unread_count": 4 }
        }))
        .unwrap();
        assert_eq!(on.remotes.unwrap().unread_count, 4);
        let off: CurrentUser = serde_json::from_value(json!({ "id": 1, "handle": "me", "remotes": null })).unwrap();
        assert!(off.remotes.is_none());
    }
}
