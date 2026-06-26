use kdl::KdlNode;

/// Directive keywords that express ordering and dependencies between units,
/// inspired by systemd unit directives (`After=`, `Before=`, `Requires=`).
///
/// These are handled centrally (see [`extract_sequencing`]) rather than by each
/// spec, so they work uniformly on every keyword.
pub const SEQUENCING_KEYWORDS: &[&str] = &["name", "after", "before", "requires"];

/// Ordering and dependency metadata parsed from a node's sequencing directives.
///
/// Names refer to other units, which default to a rule's identifier (e.g. a
/// service or package name, or a file path) unless overridden with `name`.
#[derive(Debug, Default, Clone)]
pub struct Sequencing {
    /// Explicit name for this unit; defaults to the rule identifier when absent.
    pub name: Option<String>,
    /// Units that must finish before this one runs (ordering only).
    pub after: Vec<String>,
    /// Units that this one must finish before (ordering only; the inverse of `after`).
    pub before: Vec<String>,
    /// Units that must finish successfully first. Implies `after`; if a required
    /// unit fails or is skipped, this unit is skipped too.
    pub requires: Vec<String>,
}

/// Strip sequencing directives from `node` and return the parsed [`Sequencing`].
///
/// Both forms are accepted:
/// - inline properties: `service caddy caddy.service requires=postgres after=network`
/// - child directives: `requires postgres caddy` inside the node's `{ ... }` block
///
/// After this returns, `node` contains only the arguments and child directives
/// its own spec deserializer understands, so specs need no knowledge of
/// sequencing.
pub fn extract_sequencing(node: &mut KdlNode) -> Sequencing {
    let mut seq = Sequencing::default();

    // Inline properties, e.g. `name=web requires=postgres after=network`.
    // Positional arguments (no key) are left untouched for the spec.
    node.entries_mut().retain(|entry| {
        let Some(key) = entry.name().map(|i| i.value()) else {
            return true;
        };
        let Some(value) = entry.value().as_string() else {
            return true;
        };
        match key {
            "name" => seq.name = Some(value.to_string()),
            "after" => seq.after.extend(split_names(value)),
            "before" => seq.before.extend(split_names(value)),
            "requires" => seq.requires.extend(split_names(value)),
            _ => return true,
        }
        false
    });

    // Child directives, e.g. a `{ requires postgres caddy; after network }` block.
    if let Some(children) = node.children_mut() {
        children.nodes_mut().retain(|child| {
            let names = || child.entries().iter().map(|e| e.expect_str().to_string());
            match child.name().value() {
                "name" => seq.name = child.entries().first().map(|e| e.expect_str().to_string()),
                "after" => seq.after.extend(names()),
                "before" => seq.before.extend(names()),
                "requires" => seq.requires.extend(names()),
                _ => return true,
            }
            false
        });
    }

    seq
}

/// Split a whitespace-separated inline value (`requires="postgres caddy"`) into
/// individual unit names.
fn split_names(value: &str) -> impl Iterator<Item = String> + '_ {
    value.split_whitespace().map(str::to_string)
}
