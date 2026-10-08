mod context;
mod file;
mod global_state;
mod host;
mod kdl;
mod package;
mod seq;
mod service;
#[cfg(feature = "ssh")]
pub mod ssh;
mod tombstone;
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
pub use global_state::{Schedule, State, Unit, UnitDeps};
pub use seq::{SEQUENCING_KEYWORDS, Sequencing};

use crate::{
    file::spec::FileSpec, package::spec::PackageSpec, service::spec::ServiceSpec, tombstone::Tombstone,
    user::spec::UserSpec, which::spec::WhichSpec,
};

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
    /// The rule type this rule belongs to, used to qualify unit names so that
    /// e.g. a `user` and a `service` may both be called `server`.
    ///
    /// This is the resource kind, not the config keyword that produced it: both
    /// `file` and `cp` yield rules of kind `file`, because they describe the
    /// same resource and two nodes targeting one path are a real conflict.
    fn kind(&self) -> &'static str;
    /// a unique identifier for the rule, used for debugging but not used in the implementation
    fn identifier(&self) -> &str;
    /// Units this rule needs applied first, read out of the rule's own content
    /// instead of declared in the config: a service that runs as `User=server`
    /// cannot have its working directory chowned until that account exists.
    ///
    /// References are qualified (`kind:name`) and ordering-only. Unlike an
    /// `after` the author wrote, one naming a unit the config does not declare
    /// is ignored — most `User=` accounts are the host's, not cook's to create.
    fn implied_after(&self) -> Vec<String> {
        Vec::new()
    }
    /// Units whose changes this rule reacts to, as written in the config
    /// (bare or `kind:name`). Each one is ordered before this rule, and
    /// [`RuleOverSsh::check_ssh_with`] learns whether any of them applied a
    /// change in this run. Unlike [`Rule::implied_after`], an unknown name is
    /// an error: a typo would otherwise silently never trigger.
    fn restart_on(&self) -> &[String] {
        &[]
    }
    /// check the rule
    fn check(&self) -> Result<Vec<Box<dyn Modification>>, Error>;
}

#[async_trait]
pub trait RuleOverSsh: Rule {
    /// check the rule over ssh
    #[cfg(feature = "ssh")]
    async fn check_ssh(&self, session: &crate::ssh::Session) -> Result<Vec<Box<dyn Modification>>, Error>;

    /// check the rule over ssh, given whether a unit in [`Rule::restart_on`]
    /// applied a change earlier in this run. Rules that don't react to other
    /// units ignore the flag.
    #[cfg(feature = "ssh")]
    async fn check_ssh_with(
        &self,
        session: &crate::ssh::Session,
        restart_on_changed: bool,
    ) -> Result<Vec<Box<dyn Modification>>, Error> {
        let _ = restart_on_changed;
        self.check_ssh(session).await
    }
}

/// defines how a rule will be applied to a system/resource
pub trait Modification: erased_serde::Serialize + Send + Sync + 'static {
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
    async fn apply_ssh(&self, session: std::sync::Arc<crate::ssh::Session>) -> Result<(), Error>;
}

pub type Error = Box<dyn std::error::Error + Send + Sync + 'static>;

/// Single-quote a string for safe interpolation into an `sh -c` command.
pub(crate) fn sh_single_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

pub trait VecDyn {
    fn push_boxed(&mut self, item: impl Modification + 'static);
}

impl VecDyn for Vec<Box<dyn Modification>> {
    fn push_boxed(&mut self, item: impl Modification + 'static) {
        self.push(Box::new(item));
    }
}

pub fn add_kdl_deserializers_to_context(cx: &mut Context) {
    cx.add_deserializers_for_keywords(FileSpec::kdl_keywords(), FileSpec::add_rules_to_state);
    cx.add_deserializers_for_keywords(Host::kdl_keywords(), Host::add_rules_to_state);
    cx.add_deserializers_for_keywords(ServiceSpec::kdl_keywords(), ServiceSpec::add_rules_to_state);
    cx.add_deserializers_for_keywords(UserSpec::kdl_keywords(), UserSpec::add_rules_to_state);
    cx.add_deserializers_for_keywords(WhichSpec::kdl_keywords(), WhichSpec::add_rules_to_state);
    cx.add_deserializers_for_keywords(PackageSpec::kdl_keywords(), PackageSpec::add_rules_to_state);
    cx.add_deserializers_for_keywords(Tombstone::kdl_keywords(), Tombstone::add_rules_to_state);
}
