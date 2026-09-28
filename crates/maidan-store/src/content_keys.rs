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
//! A subject once shredded stays shredded: a later event about it is sealed
//! under a throwaway key, never under a new live one, and an event that
//! arrives by federation already sealed (its origin shredded it) shreds here.

use maidan_types::{
    content_subject, seal_payload, ContentKey, ContentKeyring, Event, PeerId, WrappedKey,
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

pub(crate) fn decide(
    keys: &ContentKeyring,
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
            let key = ContentKey::generate();
            let wrapped = keys.wrap(subject, &key)?;
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

/// Unwrap the key a read joined beside an event. `None` when the event has no
/// key or the key was shredded.
pub(crate) fn unwrap_joined(
    keys: Option<&ContentKeyring>,
    subject: Option<Uuid>,
    kek_id: Option<String>,
    blob: Option<Vec<u8>>,
) -> Result<Option<ContentKey>, StoreError> {
    match (keys, subject, kek_id, blob) {
        (Some(keys), Some(subject), Some(kek_id), Some(blob)) => {
            Ok(Some(keys.unwrap(subject, &WrappedKey { kek_id, blob })?))
        }
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys() -> ContentKeyring {
        ContentKeyring::new([4; 32], Vec::new())
    }

    #[test]
    fn a_first_seal_creates_a_live_key() {
        let subject = Uuid::now_v7();
        let d = decide(&keys(), subject, KeyState::Absent, false).unwrap();
        let Write::Insert(Some(wrapped)) = &d.write else {
            panic!("expected a live insert, got {:?}", d.write);
        };
        let (key, live) = d.seal_with.as_ref().unwrap();
        assert!(live);
        assert_eq!(&keys().unwrap(subject, wrapped).unwrap(), key);
    }

    #[test]
    fn a_shredded_subject_is_sealed_under_a_throwaway_key() {
        let d = decide(&keys(), Uuid::now_v7(), KeyState::Shredded, false).unwrap();
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
        let wrapped = keys().wrap(Uuid::nil(), &ContentKey::generate()).unwrap();
        for (state, expected) in [
            (KeyState::Absent, "insert-shredded"),
            (KeyState::Live(wrapped), "shred"),
            (KeyState::Shredded, "nothing"),
        ] {
            let d = decide(&keys(), Uuid::nil(), state, true).unwrap();
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
