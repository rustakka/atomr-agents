use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

macro_rules! channel_id_newtype {
    ($name:ident, $prefix:literal) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new() -> Self {
                Self(format!("{}-{}", $prefix, Uuid::new_v4()))
            }

            #[allow(clippy::should_implement_trait)]
            pub fn from_str(s: impl Into<String>) -> Self {
                Self(s.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }

            pub fn into_string(self) -> String {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&self.0)
            }
        }

        impl From<String> for $name {
            fn from(s: String) -> Self {
                Self(s)
            }
        }

        impl From<&str> for $name {
            fn from(s: &str) -> Self {
                Self(s.to_owned())
            }
        }
    };
}

channel_id_newtype!(ChannelId, "channel");
channel_id_newtype!(ThreadId, "thread");
channel_id_newtype!(PeerId, "peer");

impl ThreadId {
    /// Stable id derived from the channel + peer pair. Two threads
    /// between the same peer and channel will collide — that is the
    /// point (a peer has one thread per channel by default).
    pub fn for_peer(channel: &ChannelId, peer: &PeerId) -> Self {
        Self(format!("{}#{}", channel.as_str(), peer.as_str()))
    }
}
