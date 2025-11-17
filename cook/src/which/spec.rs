use std::fmt::Display;

use kdl::KdlNode;
use serde::{Deserialize, Serialize};

use crate::{Context, Error, FromKdl, Modification, Rule};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhichSpec {
    pub bin: String,
    pub script: Option<String>,
    pub script_file: Option<String>,
}

impl WhichSpec {
    pub fn command(&self) -> std::process::Command {
        let mut command = std::process::Command::new("which");
        command.arg(&self.bin);
        command
    }
}

impl FromKdl for WhichSpec {
    fn kdl_keywords() -> &'static [&'static str] {
        &["which"]
    }

    fn add_rules_to_state(state: &mut crate::State, node: &KdlNode, _context: &Context) {
        let mut args = node.entries().iter();
        let bin = args.next().unwrap().expect_str().to_string();
        state.add_rule(WhichSpec {
            bin,
            script: None,
            script_file: None,
        });
    }
}

impl Rule for WhichSpec {
    fn identifier(&self) -> &str {
        &self.bin
    }

    fn check(&self) -> Result<Vec<Box<dyn Modification>>, Error> {
        let success = self.command().status()?.success();
        let result = if success {
            Vec::new()
        } else {
            let change = WhichChange::RunScript(self.script.clone().unwrap());
            let change: Box<dyn Modification> = Box::new(change);
            vec![change]
        };
        Ok(result)
    }
}

#[derive(Serialize)]
pub enum WhichChange {
    RunScript(String),
}

impl Display for WhichChange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ran script for which")
    }
}
impl Modification for WhichChange {
    fn apply(&self) -> Result<(), crate::Error> {
        todo!()
    }

    #[cfg(feature = "ssh")]
    fn downcast_ssh(&self) -> Option<&dyn crate::ModificationOverSsh> {
        Some(self)
    }

    fn fmt_human_readable(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ran script for which")
    }

    fn fmt_json(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ran script for which")
    }
}

#[cfg(feature = "ssh")]
#[async_trait::async_trait]
impl crate::ModificationOverSsh for WhichChange {
    async fn apply_ssh(&self, _session: std::sync::Arc<openssh::Session>) -> Result<(), Error> {
        todo!()
    }
}
