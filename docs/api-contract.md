# API contract

What devnull relies on from the [/dev/null server](https://github.com/dev-null-sh/dev-null.sh), so a change there is quick to find and fix. The server has no published API spec. Its source of truth is `routes/api.php`, the resources in `app/Http/Resources` (response shapes), the requests in `app/Http/Requests` (what a body may hold) and the controller tests in `tests/Feature/Controllers/Api` (worked examples, one file per controller).

Last checked against server commit `3096b3c` (27 Oct 2026). `docs/api-contract.lock` pins that commit and the server files below; `scripts/check-api.sh` compares them with the server's `main` and lists what changed since. A weekly workflow runs it.

## How devnull stays alive when the server moves

- **Unknown fields are ignored and missing ones default.** Every type in `src/model.rs` reads the fields it needs with `#[serde(default)]`, so a field the server adds costs nothing and one it drops leaves a gap, not a crash. Remotes goes further: a list or number that arrives as `null` reads as empty or zero.
- **What can't be defaulted fails with a name.** A Remotes answer that no longer parses says which one (`/dev/null changed its conversation answer (…)`), and the fix is in the matching type in `src/model.rs`.
- **Ids may be numbers or strings.** `id`, `user_id` and the like are read either way.
- **Only fixed words are matched.** The one that matters is `access` in a conversation (below). An unknown value keeps the composer closed rather than guessing.

## Signing

Every request is signed as the mobile app's are: `X-Devnull-Timestamp`, `X-Devnull-Nonce` and `X-SSH-Signature` over `timestamp\nnonce\nMETHOD\npath-with-query\nraw body` (SSHSIG, namespace `devnull-api`). See `Api::send` in `src/api.rs` and `app/Services/SignedRequests.php` on the server. Bodies are JSON.

## Endpoints

Paths are under `/api/v1`. "Reads" lists what devnull takes from the answer.

### Account, logs and commits

| Method and path | Used for |
| --- | --- |
| `GET /me` | the signed-in user, lane, pastéis, and `remotes` (null while Remotes are off: `from`, `pull_receipts`, `unread_count`) |
| `PUT /me/lane`, `PUT /me/pasteis-mode`, `PUT /me/github` | Settings |
| `POST /devices` | redeeming the sign-in code |
| `GET /log/{following,latest,trending}` (`?cursor`, `?since`) | the logs |
| `GET /users/{handle}`, `/commits` (`?replies=1`), `/followers`, `/following` | profiles |
| `POST`/`DELETE /users/{handle}/follow`, `/mute`, `/block` | people |
| `GET /commits/{id}`, `/replies` | threads |
| `POST /commits`, `DELETE /commits/{id}` | committing |
| `POST`/`DELETE /commits/{id}/star`, `POST /commits/{id}/pasteis`, `POST /commits/{id}/reports` | reactions and reports |
| `GET /starred`, `GET /notifications?type=` | starred, notifications |

### Remotes (direct messages)

| Method and path | Request | Reads |
| --- | --- | --- |
| `GET /remotes` | `tab` (open, stashed), `q`, `before` (a conversation id) | `data[]` conversations (`id`, `name`, `partner`, `last_message`, `last_message_at`, `ahead`, `is_archived`; a null `partner` is an org), `has_more`, `unread_count`, `requests_count` |
| `GET /remotes/@{handle}/messages` | `after`, `before` (message ids), `reactions_after` | `data`: `partner`, `messages[]` oldest first, `has_older`, `reactions_version`, `reaction_changes[]`, `read_through_id`, `partner_read_through_id`, `partner_read_at`, `partner_typing`, `is_archived`, `access` |
| `POST /remotes/@{handle}/messages` | `body` (max 2000), `client_id` (a UUID, so a retry doesn't duplicate), `reply_to_id` | `data`: the message |
| `PUT /remotes/@{handle}/read` | `through` (a message id) | nothing |
| `POST`/`DELETE /remotes/@{handle}/archive` | nothing | nothing (stash and unstash) |
| `POST`/`DELETE /remotes/messages/{id}/reactions` | `emoji` | `reactions[]` of the message |
| `GET /remotes/candidates` | `q` | `candidates[]`: `user`, `reason` (why they can't be messaged). Used by the New remote picker and for "Start a conversation" under an inbox search |
| `PUT /me/remote-preferences` | `from`, `pull_receipts` | nothing |

A message has `id`, `user_id`, `body`, `body_segments`, `reply_to` (`user_id`, `body`, `images_count`, `has_voice`), `images[]` (`url` is signed, so it can be fetched without a signature), `voice`, `link_preview`, `reactions[]` (`user_id`, `emoji`), `event` (orgs only) and `created_at`.

`access` says what the composer can do: `open`, `request` (they wrote first and the request isn't merged), `waiting` (the user's request isn't merged), `restricted` (they don't follow the user) or `closed` (they turned Remotes off). Only `open` lets the user write.

Not used yet: orgs (`/remotes/orgs/…`), typing (`PUT …/typing`), pings, muting, pinning, merging or dropping a fetched request, voice notes, and sending images.

Rate limits: the server groups these under `throttle:conversations` (reads and stashing) and `throttle:messages` (sending). devnull polls a conversation every 4 seconds while it is open, the inbox every 12 seconds while it is open, and the unread count once a minute otherwise.
