use serde::{Deserialize, Serialize};
use std::fmt;
use uuid::Uuid;

macro_rules! id_newtype {
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

id_newtype!(AgentId, "agent");
id_newtype!(TeamId, "team");
id_newtype!(DepartmentId, "dept");
id_newtype!(OrgId, "org");
id_newtype!(WorkflowId, "wf");
id_newtype!(HarnessId, "harness");
id_newtype!(ToolId, "tool");
id_newtype!(ToolSetId, "toolset");
id_newtype!(SkillId, "skill");
id_newtype!(PersonaId, "persona");
id_newtype!(RunId, "run");
