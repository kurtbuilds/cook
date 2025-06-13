use crate::Apply;

pub struct UserSpec {
    pub name: String,
}

pub enum UserChange {
    Add,
}

impl Apply for UserChange {
    fn apply(&self) -> Result<(), crate::Error> {
        todo!()
    }

    fn apply_ssh(&self) -> Result<(), crate::Error> {
        todo!()
    }
}
