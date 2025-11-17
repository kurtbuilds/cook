mod context;
mod file;
mod global_state;
mod host;
mod kdl;
mod package;
mod service;
mod user;
mod which;

use ::kdl::KdlNode;
use async_trait::async_trait;
pub use file::api::*;
pub use host::*;
pub use kdl::add_node;
pub use package::api::*;
pub use service::api::*;
pub use user::api::*;
pub use which::api::*;

pub use context::Context;
pub use global_state::State;

pub trait FromKdl {
    fn kdl_keywords() -> &'static [&'static str];
    /// create the spec from a kdl node
    fn add_rules_to_state(state: &mut State, node: &KdlNode, context: &Context);
}

/// defines how to interact with a rule about a system/resource
pub trait Rule: erased_serde::Serialize + std::fmt::Debug + Send + Sync + 'static {
    fn downcast_ssh(&self) -> Option<&dyn RuleOverSsh> {
        None
    }
    /// a unique identifier for the rule, used for debugging but not used in the implementation
    fn identifier(&self) -> &str;
    /// check the rule
    fn check(&self) -> Result<Vec<Box<dyn Modification>>, Error>;
}

#[async_trait]
pub trait RuleOverSsh: Rule {
    /// check the rule over ssh
    #[cfg(feature = "ssh")]
    async fn check_ssh(&self, session: &openssh::Session) -> Result<Vec<Box<dyn Modification>>, Error>;
}

/// defines how a rule will be applied to a system/resource
pub trait Modification: erased_serde::Serialize {
    fn downcast_ssh(&self) -> Option<&dyn ModificationOverSsh> {
        None
    }

    /// check the rule over ssh
    fn apply(&self) -> Result<(), Error>;

    fn fmt_human_readable(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result;

    fn fmt_json(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result;
}

#[async_trait]
pub trait ModificationOverSsh {
    #[cfg(feature = "ssh")]
    async fn apply_ssh(&self, session: std::sync::Arc<openssh::Session>) -> Result<(), Error>;
}

pub type Error = Box<dyn std::error::Error + Send + Sync + 'static>;

pub trait VecDyn {
    fn push_boxed(&mut self, item: impl Modification + 'static);
}

impl VecDyn for Vec<Box<dyn Modification>> {
    fn push_boxed(&mut self, item: impl Modification + 'static) {
        self.push(Box::new(item));
    }
}
