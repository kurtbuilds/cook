use kdl::KdlNode;

use crate::{Context, State};

pub fn add_node(node: &KdlNode, context: &Context, state: &mut State) {
    let value = node.name().value();
    for (keyword, add_rules_to_state) in context.kdl_rule_deserializers.iter() {
        if *keyword == value {
            add_rules_to_state(state, node, context);
            return;
        }
    }
    panic!("Unknown rule type: {}", value);
}
