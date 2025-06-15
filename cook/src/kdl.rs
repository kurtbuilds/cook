use kdl::KdlNode;

use crate::{Context, Host, Rule, file::spec::FileSpec};

pub fn parse_node(node: &KdlNode, context: &Context) -> Box<dyn Rule + Send + Sync + 'static> {
    let rule: Box<dyn Rule + Send + Sync + 'static> = match node.name().value() {
        "host" => Box::new(Host::from_kdl(node, context)),
        "file" | "cp" => Box::new(FileSpec::from_kdl(node, context)),
        _ => panic!("unknown rule type: {}", node.name().value()),
    };
    rule
}
