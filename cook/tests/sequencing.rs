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

/// Index of the unit with the given fully qualified `kind:name`.
fn qualified_index(state: &State, qualified: &str) -> usize {
    state
        .units()
        .iter()
        .position(|u| u.qualified() == qualified)
        .unwrap_or_else(|| panic!("no unit named {qualified}"))
}

#[test]
fn the_same_name_in_two_kinds_is_not_a_collision() {
    // A `user` and a `service` may both be called `server`: units are
    // identified by `kind:name`, so neither shadows the other.
    let state = parse("user server\nservice server \"tests/fixtures/example.service\"");
    let names: Vec<String> = state.units().iter().map(|u| u.qualified()).collect();
    assert_eq!(names, vec!["user:server", "service:server"]);
    let schedule = state.build_schedule().expect("valid schedule");
    assert_eq!(schedule.topo_order.len(), 2);
}

#[test]
fn duplicate_name_within_one_kind_is_still_rejected() {
    let state = parse("package a {\n  name dup\n}\npackage b {\n  name dup\n}");
    let err = state.build_schedule().expect_err("duplicate name should be rejected");
    assert!(
        err.to_string().contains("duplicate unit name 'package:dup'"),
        "got: {err}"
    );
}

#[test]
fn a_qualified_reference_selects_one_kind() {
    let state = parse(
        "user server\nservice server \"tests/fixtures/example.service\"\npackage web {\n  requires service:server\n}",
    );
    let schedule = state.build_schedule().expect("valid schedule");
    let web = qualified_index(&state, "package:web");
    assert_eq!(
        schedule.deps[web].requires,
        vec![qualified_index(&state, "service:server")]
    );
}

#[test]
fn an_ambiguous_bare_reference_is_rejected_and_names_the_candidates() {
    let state = parse("user server\nservice server \"tests/fixtures/example.service\"\npackage web after=server");
    let err = state.build_schedule().expect_err("ambiguous ref should be rejected");
    let err = err.to_string();
    assert!(err.contains("ambiguous"), "got: {err}");
    assert!(
        err.contains("service:server") && err.contains("user:server"),
        "got: {err}"
    );
}

#[test]
fn a_bare_reference_resolves_across_kinds_when_it_is_unique() {
    // Qualification is only required where a config is genuinely ambiguous;
    // `user server` is the only unit named `server`, so the bare name resolves.
    let state = parse("user server\npackage web after=server");
    let schedule = state.build_schedule().expect("valid schedule");
    let web = qualified_index(&state, "package:web");
    assert_eq!(schedule.deps[web].after, vec![qualified_index(&state, "user:server")]);
}

#[test]
fn a_colon_in_a_path_identifier_is_not_a_qualified_reference() {
    // File units are identified by path, and a path may contain a colon. The
    // prefix is only a rule type when it names one, so this stays a bare name.
    let state = parse("file \"/srv/a:b\"\npackage web {\n  after \"/srv/a:b\"\n}");
    let schedule = state.build_schedule().expect("valid schedule");
    let web = qualified_index(&state, "package:web");
    assert_eq!(schedule.deps[web].after, vec![qualified_index(&state, "file:/srv/a:b")]);
}

#[test]
fn a_service_runs_after_the_user_it_runs_as() {
    // Regression: the service chowns its working directory to the unit's
    // `User=` and starts under it, so declaring the account later in the file
    // must not mean creating it later in the run.
    let state = parse("service server \"tests/fixtures/example.service\"\nuser example");
    let schedule = state.build_schedule().expect("valid schedule");
    let service = qualified_index(&state, "service:server");
    let user = qualified_index(&state, "user:example");
    assert_eq!(schedule.deps[service].after, vec![user]);
    assert!(ordered_before(&schedule.topo_order, user, service));
}

#[test]
fn a_user_the_config_does_not_declare_is_not_a_dependency() {
    // `User=example` is usually an account the host already has; cook only
    // orders against it when the config manages it too.
    let state = parse("service server \"tests/fixtures/example.service\"");
    let schedule = state.build_schedule().expect("valid schedule");
    assert!(
        schedule.deps[qualified_index(&state, "service:server")]
            .after
            .is_empty()
    );
}

#[test]
#[should_panic(expected = "may not contain ':'")]
fn an_explicit_name_may_not_contain_a_colon() {
    parse("package a {\n  name we:b\n}");
}
