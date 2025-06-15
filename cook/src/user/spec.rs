use serde::Serialize;

use crate::{Error, Modification, ModificationOverSsh};
use async_trait::async_trait;

#[derive(Serialize)]
pub struct UserSpec {
    pub name: String,
    pub is_login: bool,
}

#[derive(Serialize)]
pub enum UserChange {
    Add(UserSpec),
}

impl std::fmt::Display for UserChange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UserChange::Add(_user) => write!(f, "Add"),
        }
    }
}
impl Modification for UserChange {
    fn apply(&self) -> Result<(), crate::Error> {
        todo!()
    }

    fn downcast_ssh(&self) -> Option<&dyn crate::ModificationOverSsh> {
        Some(self)
    }
}

#[async_trait]
impl ModificationOverSsh for UserChange {
    async fn apply_ssh(&self, _session: std::sync::Arc<openssh::Session>) -> Result<(), Error> {
        todo!()
    }
}
