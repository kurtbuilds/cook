//! `service <name> <unit-file>` — what cook reads out of the unit file it installs.
//!
//! systemd fails a unit at start time if its `WorkingDirectory=` does not exist,
//! so the directory — and the `User=`/`Group=` the unit's processes need on it —
//! is lifted out of the unit at parse time and becomes part of the rule. These
//! tests pin that down through the same path the CLI uses.

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

/// The rules of a state, serialized — the only public view of a rule's fields.
fn serialized(state: &State) -> String {
    let mut buf = Vec::new();
    state.serialize(&mut buf);
    String::from_utf8(buf).expect("serialized state is utf8")
}

#[test]
fn working_directory_and_its_owner_are_read_out_of_the_unit_file() {
    let state = parse(r#"service example "tests/fixtures/example.service""#);
    let json = serialized(&state);
    assert!(
        json.contains(
            r#""working_directory":{"path":"/srv/example","owner":{"user":"example","group":"example-grp"}}"#
        ),
        "expected the unit's WorkingDirectory and owner on the rule, got: {json}"
    );
}

#[test]
fn a_unit_without_a_working_directory_carries_none() {
    let state = parse(r#"service install-example "tests/fixtures/install-example.sh""#);
    assert!(
        !serialized(&state).contains("working_directory"),
        "a file with no WorkingDirectory should not produce one"
    );
}

#[test]
fn an_optional_working_directory_is_not_created() {
    // `WorkingDirectory=-/srv/optional` starts the unit even when the directory
    // is missing, so there is nothing for cook to enforce.
    let state = parse(r#"service example "tests/fixtures/optional-working-dir.service""#);
    assert!(
        !serialized(&state).contains("working_directory"),
        "a `-` prefixed WorkingDirectory should not become a rule"
    );
}
