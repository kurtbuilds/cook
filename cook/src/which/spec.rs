use serde::{Deserialize, Serialize};

use crate::{Apply, Check};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhichSpec {
    pub bin: String,
    pub script: Option<String>,
}

impl WhichSpec {
    fn command(&self) -> std::process::Command {
        let mut command = std::process::Command::new("which");
        command.arg(&self.bin);
        command
    }
}

impl Check for WhichSpec {
    fn check(&self) -> Result<Vec<Box<dyn Apply>>, crate::Error> {
        let success = self.command().status()?.success();
        let result = if success {
            Vec::new()
        } else {
            let change = WhichChange::RunScript(self.script.clone().unwrap());
            let change: Box<dyn Apply> = Box::new(change);
            vec![change]
        };
        Ok(result)
    }
    fn check_ssh(&self) -> Result<Vec<Box<dyn Apply>>, crate::Error> {
        todo!()
    }
}

pub enum WhichChange {
    RunScript(String),
}

impl Apply for WhichChange {
    fn apply(&self) -> Result<(), crate::Error> {
        todo!()
    }
    fn apply_ssh(&self) -> Result<(), crate::Error> {
        todo!()
    }
}
