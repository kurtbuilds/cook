//! `tombstone service <name>` — removes a service that an earlier config
//! installed. Cook keeps no record of what it applied, so a deleted `service`
//! line leaves its units on the host; a tombstone removes them explicitly.

use kdl::KdlNode;
use serde::{Deserialize, Serialize};

use crate::service::manager::UnitKind;
use crate::{Context, Error, FromKdl, Modification, ModificationOverSsh, Rule, RuleOverSsh, State};

#[cfg(feature = "ssh")]
use crate::service::manager::Platform;

/// Marker type for the `tombstone` keyword. Each supported kind adds its own rule.
pub struct Tombstone;

impl FromKdl for Tombstone {
    fn kdl_keywords() -> &'static [&'static str] {
        &["tombstone"]
    }

    fn add_rules_to_state(state: &mut State, node: &KdlNode, _context: &Context) {
        let entries = node.entries();
        let [kind, name] = entries else {
            panic!(
                "tombstone: expected `tombstone <kind> <name>`, got {} arguments",
                entries.len()
            );
        };
        if kind.name().is_some() || name.name().is_some() {
            panic!("tombstone: takes no options");
        }
        let name = name.expect_str().to_string();
        match kind.expect_str() {
            "service" => state.add_rule(ServiceTombstone { name }),
            other => panic!("tombstone {other} {name}: only `service` can be tombstoned"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceTombstone {
    pub name: String,
}

impl Rule for ServiceTombstone {
    /// `service`, not `tombstone`: units are identified by `kind:name`, so a
    /// config with both `service foo` and `tombstone service foo` fails as a
    /// duplicate rule.
    fn kind(&self) -> &'static str {
        "service"
    }

    fn identifier(&self) -> &str {
        &self.name
    }

    #[cfg(feature = "ssh")]
    fn downcast_ssh(&self) -> Option<&dyn RuleOverSsh> {
        Some(self)
    }

    fn check(&self) -> Result<Vec<Box<dyn Modification>>, Error> {
        todo!()
    }
}

#[cfg(feature = "ssh")]
#[async_trait::async_trait]
impl RuleOverSsh for ServiceTombstone {
    async fn check_ssh(&self, session: &crate::ssh::Session) -> Result<Vec<Box<dyn Modification>>, Error> {
        let manager = Platform::detect(session).await?.service_manager();
        let mut kinds = Vec::new();
        for kind in [UnitKind::Timer, UnitKind::Service] {
            if manager
                .remote_checksum(session, &manager.unit_path(&self.name, kind))
                .await?
                .is_some()
            {
                kinds.push(kind);
            }
        }
        if kinds.is_empty() {
            return Ok(Vec::new());
        }
        Ok(vec![Box::new(RemoveService {
            name: self.name.clone(),
            kinds,
        })])
    }
}

#[derive(Debug, Serialize)]
pub struct RemoveService {
    pub name: String,
    /// The units whose files exist, timer first so it stops before the service.
    pub kinds: Vec<UnitKind>,
}

impl Modification for RemoveService {
    #[cfg(feature = "ssh")]
    fn downcast_ssh(&self) -> Option<&dyn ModificationOverSsh> {
        Some(self)
    }

    fn apply(&self) -> Result<(), Error> {
        todo!()
    }

    fn fmt_human_readable(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "remove service {}", self.name)
    }

    fn fmt_json(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.fmt_human_readable(f)
    }
}

#[cfg(feature = "ssh")]
#[async_trait::async_trait]
impl ModificationOverSsh for RemoveService {
    async fn apply_ssh(&self, session: std::sync::Arc<crate::ssh::Session>) -> Result<(), Error> {
        let manager = Platform::detect(&session).await?.service_manager();
        manager.remove(&session, &self.name, &self.kinds).await
    }
}
