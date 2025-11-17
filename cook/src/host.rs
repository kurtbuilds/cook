use ::kdl::KdlNode;
use serde::{Deserialize, Serialize};

use crate::FromKdl;

pub fn host(hostname: impl AsRef<str>) -> Host {
    Host {
        name: hostname.as_ref().to_string(),
        roles: Vec::new(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "rule")]
pub struct Host {
    name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roles: Vec<String>,
}

impl Host {
    pub fn name(&self) -> &str {
        &self.name
    }
}
impl FromKdl for Host {
    fn kdl_keywords() -> &'static [&'static str] {
        &["host"]
    }

    fn add_rules_to_state(state: &mut crate::State, node: &KdlNode, _context: &crate::Context) {
        let entry = node.entries().iter().next().expect("No host found");
        let host = entry.value().expect_str().to_string();
        let host = Host {
            name: host,
            roles: Vec::new(),
        };
        state.add_host(host);
    }
}
