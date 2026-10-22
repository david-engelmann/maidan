//! Crypto-shredding, the backend-neutral half: what an append does with a
//! message's content key. The SQL is in `postgres::content_keys` and
//! `sqlite::content_keys`; the crypto is [`maidan_types::content_seal`].
//!
//! - `message_posted` / `message_edited`: the words are sealed under the
//!   message's key before the event is hashed. The first event creates the
//!   key; later ones reuse it.
//! - `message_tombstoned`: the key is shredded in the same transaction, along
//!   with queued deliveries that copied the words.
//!
//! A new data key is wrapped by its workspace's key, which the append opens or
//! creates in the same transaction (see [`maidan_types::ContentKeyring`] for
//! the hierarchy).
//!
//! A subject once shredded stays shredded: a later event about it is sealed
//! under a throwaway key, never under a new live one, and an event that
//! arrives by federation already sealed (its origin shredded it) shreds here.

use maidan_types::{
    content_subject, parse_workspace_kek_id, seal_payload, ContentKey, ContentKeyring, Event,
    PeerId, WrappedKey,
};
use uuid::Uuid;

use crate::error::StoreError;

/// What an append must do about content keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Plan {
    None,
    Seal {
        subject: Uuid,
        workspace_id: Uuid,
        /// The event carries its origin's `sealed` block and no words.
        arrived_shredded: bool,
    },
    Shred {
        subject: Uuid,
    },
}

impl Plan {
    /// `origin` is the federation peer an event arrived from, `None` for one
    /// written here.
    pub(crate) fn for_event(event: &Event, origin: Option<PeerId>) -> Self {
        match event {
            Event::MessagePosted {
                workspace_id,
                message,
                sealed,
                ..
            }
            | Event::MessageEdited {
                workspace_id,
                message,
                sealed,
                ..
            } => Self::Seal {
                subject: content_subject(message.id, origin),
                workspace_id: workspace_id.0,
                arrived_shredded: sealed.is_some(),
            },
            Event::MessageTombstoned { message_id, .. } => Self::Shred {
                subject: content_subject(*message_id, origin),
            },
            _ => Self::None,
        }
    }
}

/// A key row as read under lock.
#[derive(Debug)]
pub(crate) enum KeyState {
    Absent,
    Live(WrappedKey),
    Shredded,
}

impl KeyState {
    pub(crate) fn from_columns(row: Option<(Option<String>, Option<Vec<u8>>)>) -> Self {
        match row {
            None => Self::Absent,
            Some((Some(kek_id), Some(blob))) => Self::Live(WrappedKey { kek_id, blob }),
            Some(_) => Self::Shredded,
        }
    }
}

/// The row change a seal needs.
#[derive(Debug)]
pub(crate) enum Write {
    Nothing,
    /// A new row: live with this wrapped key, or born shredded.
    Insert(Option<WrappedKey>),
    Shred,
}

pub(crate) struct Decision {
    pub(crate) write: Write,
    /// The key to seal with, and whether it is the live key (handed back on
    /// the stored event) or a throwaway.
    seal_with: Option<(ContentKey, bool)>,
}

/// `workspace_key` is the workspace's active key, open in `keys`; a new data
/// key is wrapped by it, so it is required when the subject has no key yet.
pub(crate) fn decide(
    keys: &ContentKeyring,
    workspace_key: Option<Uuid>,
    subject: Uuid,
    state: KeyState,
    arrived_shredded: bool,
) -> Result<Decision, StoreError> {
    if arrived_shredded {
        let write = match state {
            KeyState::Absent => Write::Insert(None),
            KeyState::Live(_) => Write::Shred,
            KeyState::Shredded => Write::Nothing,
        };
        return Ok(Decision {
            write,
            seal_with: None,
        });
    }
    Ok(match state {
        KeyState::Absent => {
            let workspace_key = workspace_key.ok_or_else(|| {
                StoreError::InvalidInput("a new content key needs its workspace's key".into())
            })?;
            let key = ContentKey::generate();
            let wrapped = keys.wrap_for_workspace(workspace_key, subject, &key)?;
            Decision {
                write: Write::Insert(Some(wrapped)),
                seal_with: Some((key, true)),
            }
        }
        KeyState::Live(wrapped) => Decision {
            write: Write::Nothing,
            seal_with: Some((keys.unwrap(subject, &wrapped)?, true)),
        },
        KeyState::Shredded => Decision {
            write: Write::Nothing,
            seal_with: Some((ContentKey::generate(), false)),
        },
    })
}

impl Decision {
    pub(crate) fn apply(
        self,
        subject: Uuid,
        payload: &mut serde_json::Value,
    ) -> Result<Sealing, StoreError> {
        let content_key = match self.seal_with {
            None => None,
            Some((key, live)) => {
                seal_payload(payload, &key)?;
                live.then_some(key)
            }
        };
        Ok(Sealing {
            content_key_id: Some(subject),
            content_key,
        })
    }
}

/// What an append records beside the event.
#[derive(Debug, Default)]
pub(crate) struct Sealing {
    pub(crate) content_key_id: Option<Uuid>,
    pub(crate) content_key: Option<ContentKey>,
}

/// Message words are only ever written sealed. An append path without the
/// keyring cannot write them at all.
pub(crate) fn require(keys: Option<&ContentKeyring>) -> Result<&ContentKeyring, StoreError> {
    keys.ok_or_else(|| {
        StoreError::InvalidInput(
            "message content events must be appended with the content keyring".into(),
        )
    })
}

/// The workspace-key columns a read joins beside a data key (`wk_kek_id` and
/// `wk_wrapped`, from `maidan_workspace_keys wk ON wk.id = k.workspace_key_id`),
/// so the data key can be opened without a second query.
#[derive(Debug, Default)]
pub(crate) struct JoinedWorkspaceKey {
    pub(crate) kek_id: Option<String>,
    pub(crate) blob: Option<Vec<u8>>,
}

/// The stored workspace key a data key's `kek_id` names, from the columns a
/// read joined beside it. `Ok(None)` for a data key an instance KEK wrapped.
/// One whose workspace key row is gone or destroyed is an error, whatever
/// this process still holds in memory: the row is the truth.
fn joined_workspace_key(
    kek_id: &str,
    joined: JoinedWorkspaceKey,
) -> Result<Option<(Uuid, WrappedKey)>, StoreError> {
    let Some(workspace_key) = parse_workspace_kek_id(kek_id) else {
        return Ok(None);
    };
    match (joined.kek_id, joined.blob) {
        (Some(wrapping), Some(blob)) => Ok(Some((
            workspace_key,
            WrappedKey {
                kek_id: wrapping,
                blob,
            },
        ))),
        _ => Err(maidan_types::SealError::UnknownKek(kek_id.to_string()).into()),
    }
}

/// Open, through their custody, the workspace keys a batch of rows names
/// (each a data key's `kek_id` and the workspace key joined beside it). Every
/// read calls this before it maps its rows; mapping then only checks the key
/// is open ([`unwrap_joined`]). A row whose workspace key is gone is skipped
/// here and fails when it is mapped.
pub(crate) async fn open_workspace_keys(
    keys: &ContentKeyring,
    rows: impl IntoIterator<Item = (Option<String>, JoinedWorkspaceKey)>,
) -> Result<(), StoreError> {
    let stored: Vec<(Uuid, WrappedKey)> = rows
        .into_iter()
        .filter_map(|(kek_id, joined)| joined_workspace_key(kek_id.as_deref()?, joined).ok()?)
        .collect();
    keys.open_workspace_keys(stored).await?;
    Ok(())
}

/// Open the workspace key one data key needs; see [`open_workspace_keys`].
pub(crate) async fn open_joined(
    keys: &ContentKeyring,
    kek_id: &str,
    joined: JoinedWorkspaceKey,
) -> Result<(), StoreError> {
    if let Some(stored) = joined_workspace_key(kek_id, joined)? {
        keys.open_workspace_keys([stored]).await?;
    }
    Ok(())
}

/// Fail unless the workspace key a data key names is open from the stored
/// form joined beside it.
fn require_joined(
    keys: &ContentKeyring,
    kek_id: &str,
    joined: JoinedWorkspaceKey,
) -> Result<(), StoreError> {
    if let Some((id, wrapped)) = joined_workspace_key(kek_id, joined)? {
        keys.require_open(id, &wrapped)?;
    }
    Ok(())
}

/// Unwrap the key a read joined beside an event, its workspace key already
/// opened by [`open_workspace_keys`]. `None` when the event has no key or the
/// key was shredded.
pub(crate) fn unwrap_joined(
    keys: Option<&ContentKeyring>,
    subject: Option<Uuid>,
    kek_id: Option<String>,
    blob: Option<Vec<u8>>,
    joined: JoinedWorkspaceKey,
) -> Result<Option<ContentKey>, StoreError> {
    match (keys, subject, kek_id, blob) {
        (Some(keys), Some(subject), Some(kek_id), Some(blob)) => {
            require_joined(keys, &kek_id, joined)?;
            Ok(Some(keys.unwrap(subject, &WrappedKey { kek_id, blob })?))
        }
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create(keys: &ContentKeyring, id: Uuid) {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(keys.create_workspace_key(id))
            .unwrap();
    }

    /// A keyring with one open workspace key.
    fn keys() -> (ContentKeyring, Uuid) {
        let keys = ContentKeyring::new([4; 32], Vec::new());
        let workspace_key = Uuid::now_v7();
        create(&keys, workspace_key);
        (keys, workspace_key)
    }

    #[test]
    fn a_first_seal_creates_a_live_key_under_the_workspace_key() {
        let (keys, workspace_key) = keys();
        let subject = Uuid::now_v7();
        let d = decide(&keys, Some(workspace_key), subject, KeyState::Absent, false).unwrap();
        let Write::Insert(Some(wrapped)) = &d.write else {
            panic!("expected a live insert, got {:?}", d.write);
        };
        assert_eq!(
            wrapped.kek_id,
            maidan_types::workspace_kek_id(workspace_key)
        );
        let (key, live) = d.seal_with.as_ref().unwrap();
        assert!(live);
        assert_eq!(&keys.unwrap(subject, wrapped).unwrap(), key);
    }

    #[test]
    fn a_data_key_under_one_workspace_key_does_not_open_under_another() {
        let (keys, mine) = keys();
        let theirs = Uuid::now_v7();
        create(&keys, theirs);
        let subject = Uuid::now_v7();
        let wrapped = keys
            .wrap_for_workspace(mine, subject, &ContentKey::generate())
            .unwrap();
        // The same bytes, claimed to be under the other workspace's key.
        let moved = WrappedKey {
            kek_id: maidan_types::workspace_kek_id(theirs),
            blob: wrapped.blob,
        };
        assert!(keys.unwrap(subject, &moved).is_err());
    }

    #[test]
    fn a_shredded_subject_is_sealed_under_a_throwaway_key() {
        let (keys, workspace_key) = keys();
        let d = decide(
            &keys,
            Some(workspace_key),
            Uuid::now_v7(),
            KeyState::Shredded,
            false,
        )
        .unwrap();
        assert!(matches!(d.write, Write::Nothing));
        assert!(!d.seal_with.as_ref().unwrap().1);
        let mut payload = serde_json::json!({
            "kind": "message_edited",
            "message": {"id": Uuid::now_v7(), "body": "words"}
        });
        let sealing = d.apply(Uuid::now_v7(), &mut payload).unwrap();
        assert!(sealing.content_key.is_none());
        assert!(!payload.to_string().contains("words"));
    }

    #[test]
    fn an_event_that_arrives_shredded_shreds_here() {
        let (keys, workspace_key) = keys();
        let wrapped = keys.wrap(Uuid::nil(), &ContentKey::generate()).unwrap();
        for (state, expected) in [
            (KeyState::Absent, "insert-shredded"),
            (KeyState::Live(wrapped), "shred"),
            (KeyState::Shredded, "nothing"),
        ] {
            let d = decide(&keys, Some(workspace_key), Uuid::nil(), state, true).unwrap();
            let got = match d.write {
                Write::Insert(None) => "insert-shredded",
                Write::Shred => "shred",
                Write::Nothing => "nothing",
                Write::Insert(Some(_)) => "insert-live",
            };
            assert_eq!(got, expected);
            assert!(d.seal_with.is_none());
        }
    }

    #[test]
    fn appending_words_without_the_keyring_is_refused() {
        assert!(matches!(require(None), Err(StoreError::InvalidInput(_))));
    }
}
