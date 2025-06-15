pub use ::kdl::KdlDocument;
use cook::{Context, State};

pub fn parse_kdl(content: &str, context: Context) -> State {
    let mut state = State::new();
    let doc = match KdlDocument::parse(content) {
        Ok(doc) => doc,
        Err(err) => {
            for d in &err.diagnostics {
                if let Some(path) = &context.file {
                    println!("{}: {}", path.display(), d);
                } else {
                    println!("{}", d);
                }
            }
            panic!("Failed to parse KDL document");
        }
    };
    for node in doc.nodes() {
        let node = cook::parse_node(node, &context);
        state.add_rule(node);
    }
    state
}
