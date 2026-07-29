//! `which <bin> script="..."` — installing a binary that isn't already present.
//!
//! The parser used to drop the `script`/`script_file` properties entirely, which
//! made `check` panic on exactly the case the directive exists for: a missing
//! binary. These tests pin the property down to the produced modification.

use cook::{Context, State, add_kdl_deserializers_to_context, add_node};
use kdl::KdlDocument;

/// Parse a KDL config into a [`State`], exercising the same path the CLI uses.
fn parse(src: &str) -> State {
    let mut context = Context::new(".");
    add_kdl_deserializers_to_context(&mut context);
    let mut state = State::new();
    let doc = KdlDocument::parse(src).expect("valid kdl");
    for node in doc.nodes() {
        add_node(node, &context, &mut state);
    }
    state
}

/// A binary name no host will have, so `check` always takes the missing branch.
const ABSENT: &str = "cook-test-definitely-absent-binary";

#[test]
fn missing_binary_produces_a_change() {
    let state = parse(&format!(r#"which {ABSENT} script="echo hi""#));
    // Errors if the script was dropped during parsing — there'd be nothing to run.
    let changes = state.rules()[0].check().expect("check should find the script");
    assert_eq!(changes.len(), 1);
}

#[test]
fn present_binary_produces_no_change() {
    // `sh` is on PATH everywhere cook runs.
    let state = parse(r#"which sh script="exit 1""#);
    let changes = state.rules()[0].check().expect("check succeeds");
    assert!(changes.is_empty());
}

#[test]
fn script_file_is_read_relative_to_the_config_root() {
    // Read at parse time, since apply may run somewhere the file doesn't exist.
    let state = parse(&format!(
        r#"which {ABSENT} script_file="tests/fixtures/install-example.sh""#
    ));
    let changes = state.rules()[0].check().expect("check should find the script");
    assert_eq!(changes.len(), 1);
}

#[test]
#[should_panic(expected = "specify `script` or `script_file`")]
fn script_is_required() {
    parse(&format!("which {ABSENT}"));
}
