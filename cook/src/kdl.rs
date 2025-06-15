use kdl::KdlNode;

use crate::{Context, Host, Rule, State, file::spec::FileSpec};

pub fn add_node(node: &KdlNode, context: &Context, state: &mut State) {
    match node.name().value() {
        "host" => {
            let host = Host::from_kdl(node);
            state.add_host(host);
        }
        "file" | "cp" => state.add_rule(FileSpec::from_kdl(node, context)),
        _ => panic!("unknown rule type: {}", node.name().value()),
    }
}
