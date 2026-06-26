//! Tests for systemd-inspired sequencing directives (name/after/before/requires)
//! and the resulting execution schedule.

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

/// Index of the unit with the given name.
fn unit_index(state: &State, name: &str) -> usize {
    state
        .units()
        .iter()
        .position(|u| u.name == name)
        .unwrap_or_else(|| panic!("no unit named {name}"))
}

/// Assert `topo` lists `before` ahead of `after`.
fn ordered_before(topo: &[usize], before: usize, after: usize) -> bool {
    let pos = |x| topo.iter().position(|&u| u == x).unwrap();
    pos(before) < pos(after)
}

#[test]
fn unit_defaults_to_rule_identifier() {
    let state = parse("package alpha\npackage beta");
    let names: Vec<&str> = state.units().iter().map(|u| u.name.as_str()).collect();
    assert_eq!(names, vec!["alpha", "beta"]);
}

#[test]
fn one_node_with_many_rules_is_one_unit() {
    // `package foo bar` adds two rules but is a single schedulable unit.
    let state = parse("package foo bar");
    assert_eq!(state.units().len(), 1);
    assert_eq!(state.units()[0].rules, 0..2);
    assert_eq!(state.units()[0].name, "foo");
}

#[test]
fn after_child_directive_creates_ordering() {
    let state = parse("package alpha\npackage beta {\n  after alpha\n}");
    let alpha = unit_index(&state, "alpha");
    let beta = unit_index(&state, "beta");
    let schedule = state.build_schedule().expect("valid schedule");
    assert_eq!(schedule.deps[beta].after, vec![alpha]);
    assert!(schedule.deps[beta].requires.is_empty());
    assert!(ordered_before(&schedule.topo_order, alpha, beta));
}

#[test]
fn after_inline_property_creates_ordering() {
    // The inline `after=alpha` must be stripped before `package` parses its args,
    // otherwise it would be mistaken for a package name.
    let state = parse("package alpha\npackage beta after=alpha");
    let beta = unit_index(&state, "beta");
    // beta's spec only saw the positional arg, so its single rule is named beta.
    assert_eq!(state.rules().len(), 2);
    let schedule = state.build_schedule().expect("valid schedule");
    assert_eq!(schedule.deps[beta].after, vec![unit_index(&state, "alpha")]);
}

#[test]
fn before_is_inverted_into_after() {
    let state = parse("package alpha {\n  before beta\n}\npackage beta");
    let alpha = unit_index(&state, "alpha");
    let beta = unit_index(&state, "beta");
    let schedule = state.build_schedule().expect("valid schedule");
    // `alpha before beta` => beta runs after alpha.
    assert_eq!(schedule.deps[beta].after, vec![alpha]);
    assert!(ordered_before(&schedule.topo_order, alpha, beta));
}

#[test]
fn requires_implies_after_and_is_tracked() {
    let state = parse("package db\npackage web {\n  requires db\n}");
    let db = unit_index(&state, "db");
    let web = unit_index(&state, "web");
    let schedule = state.build_schedule().expect("valid schedule");
    assert_eq!(schedule.deps[web].after, vec![db]);
    assert_eq!(schedule.deps[web].requires, vec![db]);
}

#[test]
fn name_overrides_reference() {
    let state = parse("package postgres {\n  name db\n}\npackage web {\n  requires db\n}");
    assert!(state.units().iter().any(|u| u.name == "db"));
    let schedule = state.build_schedule().expect("valid schedule");
    let db = unit_index(&state, "db");
    let web = unit_index(&state, "web");
    assert_eq!(schedule.deps[web].requires, vec![db]);
}

#[test]
fn multiple_dependencies_in_one_directive() {
    let state = parse("package a\npackage b\npackage c {\n  requires a b\n}");
    let c = unit_index(&state, "c");
    let schedule = state.build_schedule().expect("valid schedule");
    let mut expected = vec![unit_index(&state, "a"), unit_index(&state, "b")];
    expected.sort();
    assert_eq!(schedule.deps[c].after, expected);
    assert_eq!(schedule.deps[c].requires, expected);
}

#[test]
fn cycle_is_rejected() {
    let state = parse("package a {\n  after b\n}\npackage b {\n  after a\n}");
    let err = state.build_schedule().expect_err("cycle should be rejected");
    assert!(err.to_string().contains("cycle"), "got: {err}");
}

#[test]
fn unknown_reference_is_rejected() {
    let state = parse("package a {\n  requires nope\n}");
    let err = state.build_schedule().expect_err("unknown ref should be rejected");
    assert!(err.to_string().contains("unknown unit"), "got: {err}");
}

#[test]
fn duplicate_name_is_rejected() {
    let state = parse("package a {\n  name dup\n}\npackage b {\n  name dup\n}");
    let err = state.build_schedule().expect_err("duplicate name should be rejected");
    assert!(err.to_string().contains("duplicate"), "got: {err}");
}

#[test]
fn self_dependency_is_rejected() {
    let state = parse("package a {\n  requires a\n}");
    let err = state.build_schedule().expect_err("self dep should be rejected");
    assert!(err.to_string().contains("itself"), "got: {err}");
}

#[test]
fn independent_units_have_no_dependencies() {
    let state = parse("package a\npackage b\npackage c");
    let schedule = state.build_schedule().expect("valid schedule");
    assert!(schedule.deps.iter().all(|d| d.after.is_empty()));
    assert_eq!(schedule.topo_order.len(), 3);
}
