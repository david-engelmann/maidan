//! Typed ID wrappers. Each domain entity gets its own newtype so the type
//! system catches "I passed a ThreadId where a MemberId was expected".

use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! id_newtype {
    ($name:ident) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub Uuid);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v7(uuid::Timestamp::now(uuid::NoContext)))
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}

id_newtype!(MemberId);
id_newtype!(ChannelId);
id_newtype!(ThreadId);
id_newtype!(MessageId);
id_newtype!(ArtifactId);
id_newtype!(WorkspaceId);
id_newtype!(ApiTokenId);
id_newtype!(PeerId);
