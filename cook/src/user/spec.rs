use kdl::KdlNode;
use serde::Serialize;

use crate::{Context, Error, FromKdl, Modification, ModificationOverSsh, Rule, RuleOverSsh, State};

#[derive(Debug, Serialize)]
pub struct UserSpec {
    pub name: String,
    pub is_login: bool,
}

impl FromKdl for UserSpec {
    fn kdl_keywords() -> &'static [&'static str] {
        &["user"]
    }

    fn add_rules_to_state(state: &mut State, node: &KdlNode, _context: &Context) {
        let mut args = node.entries().iter();
        let name = args.next().unwrap().expect_str().to_string();
        let mut spec = UserSpec { name, is_login: false };
        while let Some(e) = args.next() {
            match e.expect_str() {
                "is_login" => spec.is_login = true,
                z => panic!("Unrecognized keyword for user declaration: {}", z),
            }
        }
        state.add_rule(spec);
    }
}

impl Rule for UserSpec {
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
impl RuleOverSsh for UserSpec {
    async fn check_ssh(&self, session: &openssh::Session) -> Result<Vec<Box<dyn Modification>>, Error> {
        let output = session.command("id").arg(&self.name).output().await?;
        if output.status.success() {
            Ok(vec![])
        } else {
            // User doesn't exist, need to add them
            Ok(vec![Box::new(UserChange::Add(UserSpec {
                name: self.name.clone(),
                is_login: self.is_login,
            }))])
        }
    }
}

#[derive(Serialize)]
pub enum UserChange {
    Add(UserSpec),
}

impl Modification for UserChange {
    fn apply(&self) -> Result<(), Error> {
        todo!()
    }

    #[cfg(feature = "ssh")]
    fn downcast_ssh(&self) -> Option<&dyn crate::ModificationOverSsh> {
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
impl ModificationOverSsh for UserChange {
    async fn apply_ssh(&self, _session: std::sync::Arc<openssh::Session>) -> Result<(), Error> {
        match self {
            UserChange::Add(user_spec) => {
                let mut cmd = _session.command("useradd");
                if user_spec.is_login {
                    cmd.arg("-m");
                }
                cmd.arg(&user_spec.name);
                let output = cmd.output().await?;
                if !output.status.success() {
                    return Err(anyhow::anyhow!("Failed to add user").into_boxed_dyn_error());
                }
                Ok(())
            }
        }
    }
}
