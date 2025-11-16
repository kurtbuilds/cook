use std::fmt::Display;

use kdl::KdlNode;
use serde::{Deserialize, Serialize};

use crate::{Context, Error, Modification, Rule};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhichSpec {
    pub bin: String,
    pub script: Option<String>,
    pub script_file: Option<String>,
}

impl WhichSpec {
    fn command(&self) -> std::process::Command {
        let mut command = std::process::Command::new("which");
        command.arg(&self.bin);
        command
    }
}

impl Rule for WhichSpec {
    fn identifier(&self) -> &str {
        &self.bin
    }

    fn from_kdl(node: &KdlNode, _context: &Context) -> Self {
        assert_eq!(node.name().value(), "which");
        let mut args = node.entries().iter();
        let bin = args
            .next()
            .unwrap()
            .value()
            .as_string()
            .expect("Expected a string")
            .to_string();
        let entry = args
            .next()
            .expect("which requires script or script_file to create the executable");
        WhichSpec {
            bin,
            script: None,
            script_file: None,
        }
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
}

#[cfg(feature = "ssh")]
#[async_trait::async_trait]
impl crate::ModificationOverSsh for WhichChange {
    async fn apply_ssh(&self, _session: std::sync::Arc<openssh::Session>) -> Result<(), Error> {
        todo!()
    }
}
