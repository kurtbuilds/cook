use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use kdl::KdlNode;

use crate::State;

pub struct Context {
    root: PathBuf,
    pub(crate) kdl_rule_deserializers: BTreeMap<&'static str, fn(&mut State, &KdlNode, &Context)>,
}

impl Context {
    pub(crate) fn add_deserializers_for_keywords(
        &mut self,
        keywords: &[&'static str],
        f: fn(&mut State, &KdlNode, &Context),
    ) {
        for keyword in keywords {
            self.kdl_rule_deserializers.insert(*keyword, f);
        }
    }

    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let root = fs::canonicalize(root).expect("Failed to canonicalize path");
        Context {
            root,
            // file,
            kdl_rule_deserializers: BTreeMap::new(),
        }
    }

    pub fn local_path(&self, path: impl AsRef<Path>) -> PathBuf {
        self.root.join(path.as_ref())
    }
}
