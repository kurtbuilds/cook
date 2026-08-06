use std::fmt::Display;

use kdl::KdlNode;
use serde::{Deserialize, Serialize};

use crate::{Context, Error, FromKdl, Modification, Rule, RuleOverSsh};

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

    /// The change to apply when `bin` is missing. The Cookfile parser rejects a
    /// rule with no script, but the field stays optional on the struct (it is
    /// also built programmatically and deserialized by the agent), so report a
    /// missing one as a configuration error rather than panicking here.
    fn run_script_change(&self) -> Result<Box<dyn Modification>, Error> {
        let script = self.script.clone().ok_or_else(|| {
            format!(
                "which {}: no script to run, and {} is not installed",
                self.bin, self.bin
            )
        })?;
        Ok(Box::new(WhichChange::RunScript {
            bin: self.bin.clone(),
            script,
        }))
    }
}

impl FromKdl for WhichSpec {
    fn kdl_keywords() -> &'static [&'static str] {
        &["which"]
    }

    fn add_rules_to_state(state: &mut crate::State, node: &KdlNode, context: &Context) {
        let mut entries = node.entries().iter();
        let bin = entries
            .next()
            .expect("which requires a binary name")
            .expect_str()
            .to_string();

        // `script_file` is read here rather than at check time: the path is
        // relative to the local Cookfile, and `check`/`apply` may run on the
        // remote (or in the agent), where that file doesn't exist. Reading now
        // also fails fast on a missing file, like `service`'s `timer=`.
        let mut script = None;
        let mut script_file = None;
        for e in entries {
            match e.name().expect("Failed to get node name").value() {
                "script" => script = Some(e.expect_str().to_string()),
                "script_file" => {
                    let path = e.expect_str().to_string();
                    let content = std::fs::read_to_string(context.local_path(&path))
                        .unwrap_or_else(|e| panic!("which {bin}: failed to read {path}: {e}"));
                    script = Some(content);
                    script_file = Some(path);
                }
                z => panic!("Unexpected option for which: {}", z),
            }
        }

        if script.is_none() {
            panic!("which {bin}: specify `script` or `script_file` — there is nothing to run if {bin} is missing");
        }

        state.add_rule(WhichSpec {
            bin,
            script,
            script_file,
        });
    }
}

impl Rule for WhichSpec {
    #[cfg(feature = "ssh")]
    fn downcast_ssh(&self) -> Option<&dyn crate::RuleOverSsh> {
        Some(self)
    }
    fn kind(&self) -> &'static str {
        "which"
    }

    fn identifier(&self) -> &str {
        &self.bin
    }

    fn check(&self) -> Result<Vec<Box<dyn Modification>>, Error> {
        let success = self.command().status()?.success();
        let result = if success {
            Vec::new()
        } else {
            vec![self.run_script_change()?]
        };
        Ok(result)
    }
}

#[cfg(feature = "ssh")]
#[async_trait::async_trait]
impl RuleOverSsh for WhichSpec {
    async fn check_ssh(&self, session: &openssh::Session) -> Result<Vec<Box<dyn Modification>>, Error> {
        let mut changes: Vec<Box<dyn Modification>> = Vec::new();
        let success = session.command("which").arg(&self.bin).output().await?.status.success();
        if !success {
            changes.push(self.run_script_change()?);
        }
        Ok(changes)
    }
}

#[derive(Serialize)]
pub enum WhichChange {
    RunScript { bin: String, script: String },
}

impl Display for WhichChange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WhichChange::RunScript { bin, .. } => write!(f, "installed {bin}"),
        }
    }
}
impl Modification for WhichChange {
    fn apply(&self) -> Result<(), crate::Error> {
        let WhichChange::RunScript { bin, script } = self;
        let status = std::process::Command::new("sh").arg("-c").arg(script).status()?;
        if !status.success() {
            return Err(format!("which {bin}: install script exited with {status}").into());
        }
        Ok(())
    }

    #[cfg(feature = "ssh")]
    fn downcast_ssh(&self) -> Option<&dyn crate::ModificationOverSsh> {
        Some(self)
    }

    fn fmt_human_readable(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Display::fmt(self, f)
    }

    fn fmt_json(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Display::fmt(self, f)
    }
}

#[cfg(feature = "ssh")]
#[async_trait::async_trait]
impl crate::ModificationOverSsh for WhichChange {
    async fn apply_ssh(&self, session: std::sync::Arc<openssh::Session>) -> Result<(), Error> {
        let WhichChange::RunScript { bin, script } = self;
        // Install scripts are multi-line shell, so they go through `sh -c`
        // rather than being split into a command + args.
        let output = session.command("sh").arg("-c").arg(script).output().await?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(format!("which {bin}: install script failed: {}", stderr.trim()).into());
        }
        Ok(())
    }
}
