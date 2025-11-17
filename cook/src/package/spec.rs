use kdl::KdlNode;
use serde::Serialize;

use crate::{Context, Error, FromKdl, Modification, ModificationOverSsh, Rule, RuleOverSsh, State};

#[derive(Debug, Serialize)]
pub struct PackageSpec {
    pub name: String,
}

impl FromKdl for PackageSpec {
    fn kdl_keywords() -> &'static [&'static str] {
        &["package"]
    }

    fn add_rules_to_state(state: &mut State, node: &KdlNode, _context: &Context) {
        let mut args = node.entries().iter();
        while let Some(p) = args.next() {
            let name = p.expect_str().to_string();
            let spec = PackageSpec { name };
            state.add_rule(spec);
        }
    }
}

impl Rule for PackageSpec {
    #[cfg(feature = "ssh")]
    fn downcast_ssh(&self) -> Option<&dyn crate::RuleOverSsh> {
        Some(self)
    }
    fn identifier(&self) -> &str {
        self.name.as_str()
    }

    fn check(&self) -> Result<Vec<Box<dyn Modification>>, Error> {
        todo!()
    }
}

#[cfg(feature = "ssh")]
#[async_trait::async_trait]
impl RuleOverSsh for PackageSpec {
    async fn check_ssh(&self, session: &openssh::Session) -> Result<Vec<Box<dyn Modification>>, Error> {
        let output = session
            .command("apt")
            .arg("-qq")
            .arg("list")
            .arg(&self.name)
            .output()
            .await?;
        let stdout = String::from_utf8(output.stdout).expect("Not utf8 output");
        if stdout.contains(&self.name) {
            Ok(vec![])
        } else {
            // User doesn't exist, need to add them
            Ok(vec![Box::new(PackageChange::AddPackage(PackageSpec {
                name: self.name.clone(),
            }))])
        }
    }
}

#[derive(Serialize)]
pub enum PackageChange {
    AddPackage(PackageSpec),
}

impl Modification for PackageChange {
    fn apply(&self) -> Result<(), Error> {
        todo!()
    }

    #[cfg(feature = "ssh")]
    fn downcast_ssh(&self) -> Option<&dyn ModificationOverSsh> {
        Some(self)
    }

    fn fmt_human_readable(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "user change")
    }

    fn fmt_json(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "user change")
    }
}

#[cfg(feature = "ssh")]
#[async_trait::async_trait]
impl ModificationOverSsh for PackageChange {
    async fn apply_ssh(&self, session: std::sync::Arc<openssh::Session>) -> Result<(), Error> {
        match self {
            PackageChange::AddPackage(spec) => {
                let success = session
                    .command("apt")
                    .arg("install")
                    .arg("-y")
                    .arg(&spec.name)
                    .output()
                    .await?
                    .status
                    .success();
                if !success {
                    return Err(anyhow::anyhow!("Failed to install package").into_boxed_dyn_error());
                }
                Ok(())
            }
        }
    }
}
