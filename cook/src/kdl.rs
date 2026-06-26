use kdl::KdlNode;

use crate::{Context, State, seq::extract_sequencing};

pub fn add_node(node: &KdlNode, context: &Context, state: &mut State) {
    // Pull out sequencing directives (name/after/before/requires) before the
    // spec sees the node, so individual specs don't need to know about them.
    let mut node = node.clone();
    let sequencing = extract_sequencing(&mut node);

    let value = node.name().value();
    let start = state.rules().len();
    let mut handled = false;
    for (keyword, add_rules_to_state) in context.kdl_rule_deserializers.iter() {
        if *keyword == value {
            add_rules_to_state(state, &node, context);
            handled = true;
            break;
        }
    }
    if !handled {
        panic!("Unknown rule type: {}", value);
    }

    // Everything the node produced becomes one schedulable unit.
    let end = state.rules().len();
    state.add_unit(sequencing, start..end);
}
