use std::{fs, path::PathBuf};

pub struct Context {
    // expect that root is already canonicalized
    pub root: PathBuf,
    pub file: Option<PathBuf>,
}

impl Context {
    pub fn new(root: impl Into<PathBuf>, file: Option<PathBuf>) -> Self {
        let root = root.into();
        let root = fs::canonicalize(root).expect("Failed to canonicalize path");
        Context { root, file }
    }
}
