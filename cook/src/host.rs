use ::kdl::KdlNode;
use serde::{Deserialize, Serialize};

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
    pub fn from_kdl(node: &KdlNode) -> Self {
        assert_eq!(node.name().value(), "host");
        let entry = node.entry(0).expect("No host found");
        let host = entry
            .value()
            .as_string()
            .expect("Host is not a string")
            .to_string();
        Host {
            name: host,
            roles: Vec::new(),
        }
    }
}
