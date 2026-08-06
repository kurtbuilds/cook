// use std::{io::Write, sync::Mutex};

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    ops::Range,
};

use crate::{Host, Rule, Sequencing};

/// A schedulable group of rules produced by a single config node, plus the
/// ordering/dependency edges declared on it. Units are the granularity at which
/// sequencing is expressed and enforced.
#[derive(Debug)]
pub struct Unit {
    /// Rule type of the unit's first rule. Together with `name` this forms the
    /// unit's identity, so a `user` and a `service` may share a name.
    pub kind: &'static str,
    /// Name other units reference in `after`/`before`/`requires`. Defaults to
    /// the identifier of the unit's first rule.
    pub name: String,
    /// Range of [`State::rules`] indices owned by this unit.
    pub rules: Range<usize>,
    pub after: Vec<String>,
    pub before: Vec<String>,
    pub requires: Vec<String>,
}

impl Unit {
    /// The unit's fully qualified name, `kind:name`. Unique across a config, and
    /// the form a reference must take when a bare name is ambiguous.
    pub fn qualified(&self) -> String {
        format!("{}:{}", self.kind, self.name)
    }
}

/// Resolved dependencies for one unit, by unit index.
#[derive(Debug)]
pub struct UnitDeps {
    /// Units that must finish before this one runs (ordering). Includes `requires`.
    pub after: Vec<usize>,
    /// Units whose failure/skip causes this unit to be skipped (subset of `after`).
    pub requires: Vec<usize>,
}

/// A validated execution plan over [`State::units`].
#[derive(Debug)]
pub struct Schedule {
    /// Unit indices in dependency order: a unit always appears after every unit
    /// it depends on. Lets a scheduler build per-unit futures referencing their
    /// already-built dependencies.
    pub topo_order: Vec<usize>,
    /// Per-unit resolved dependencies, indexed by unit index.
    pub deps: Vec<UnitDeps>,
}

#[derive(Debug)]
pub struct State {
    // rules that are applied to infra. e.g. dns, network, node/host existence, etc.
    _infra_rules: Vec<Box<dyn Rule>>,
    // rules that are applied to hosts. e.g. package installation, ssh, sudo, etc.
    // TODO add regex rules to make sure we only apply rules to hosts that match certain patterns
    host_rules: Vec<Box<dyn Rule>>,
    // schedulable units over `host_rules`, one per config node that produced rules
    units: Vec<Unit>,
    hosts: Vec<Host>,
}

impl State {
    pub const fn new() -> Self {
        Self {
            host_rules: Vec::new(),
            units: Vec::new(),
            hosts: Vec::new(),
            _infra_rules: Vec::new(),
        }
    }

    pub fn add_host(&mut self, host: Host) {
        self.hosts.push(host);
    }

    pub fn rules(&self) -> &[Box<dyn Rule>] {
        &self.host_rules
    }

    pub fn units(&self) -> &[Unit] {
        &self.units
    }

    /// Register the rules added for one config node as a unit, attaching its
    /// sequencing directives. Nodes that produced no host rules (e.g. `host`)
    /// are not schedulable and are skipped.
    pub fn add_unit(&mut self, sequencing: Sequencing, rules: Range<usize>) {
        if rules.is_empty() {
            return;
        }
        let first = &self.host_rules[rules.start];
        let kind = first.kind();
        let name = sequencing.name.unwrap_or_else(|| first.identifier().to_string());
        self.units.push(Unit {
            kind,
            name,
            rules,
            after: sequencing.after,
            before: sequencing.before,
            requires: sequencing.requires,
        });
    }

    /// Resolve unit names to indices, invert `before` edges into `after` edges,
    /// validate references, detect cycles, and produce a topological order.
    ///
    /// Units are identified by `kind:name`, so a `user` and a `service` may
    /// share a name. References may be written either way: a bare name resolves
    /// as long as exactly one unit answers to it, which keeps the qualified form
    /// necessary only where a config is genuinely ambiguous.
    ///
    /// Errors on two units of the same kind sharing a name, references to
    /// unknown or ambiguous units, self dependencies, and dependency cycles.
    pub fn build_schedule(&self) -> Result<Schedule, crate::Error> {
        let n = self.units.len();

        let mut qualified: BTreeMap<(&str, &str), usize> = BTreeMap::new();
        let mut bare: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
        let mut kinds: BTreeSet<&str> = BTreeSet::new();
        for (i, unit) in self.units.iter().enumerate() {
            if qualified.insert((unit.kind, unit.name.as_str()), i).is_some() {
                return Err(anyhow::anyhow!("duplicate unit name '{}'", unit.qualified()).into());
            }
            bare.entry(unit.name.as_str()).or_default().push(i);
            kinds.insert(unit.kind);
        }

        let resolve = |referrer: &Unit, name: &str| -> Result<usize, crate::Error> {
            let unknown = || anyhow::anyhow!("unit '{}' references unknown unit '{name}'", referrer.qualified()).into();
            // Treat a `kind:name` reference as qualified only when the prefix is
            // a rule type in play: file units are identified by path, and a path
            // may legitimately contain a colon.
            if let Some((kind, unqualified)) = name.split_once(':')
                && kinds.contains(kind)
            {
                return qualified.get(&(kind, unqualified)).copied().ok_or_else(unknown);
            }
            match bare.get(name).map(Vec::as_slice) {
                Some([unit]) => Ok(*unit),
                // Zero candidates cannot occur: names are only inserted with a unit.
                Some(candidates) => {
                    let candidates: Vec<String> = candidates.iter().map(|&u| self.units[u].qualified()).collect();
                    Err(anyhow::anyhow!(
                        "unit '{}' references '{name}', which is ambiguous between {}; qualify the reference",
                        referrer.qualified(),
                        candidates.join(", ")
                    )
                    .into())
                }
                None => Err(unknown()),
            }
        };

        // edges[u] = units that must run before u. requires[u] ⊆ edges[u].
        let mut edges: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); n];
        let mut requires: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); n];
        for (u, unit) in self.units.iter().enumerate() {
            for name in &unit.after {
                edges[u].insert(resolve(unit, name)?);
            }
            for name in &unit.requires {
                let dep = resolve(unit, name)?;
                edges[u].insert(dep);
                requires[u].insert(dep);
            }
            for name in &unit.before {
                // `u before x` means x runs after u.
                let target = resolve(unit, name)?;
                edges[target].insert(u);
            }
        }
        for (u, unit) in self.units.iter().enumerate() {
            if edges[u].contains(&u) {
                return Err(anyhow::anyhow!("unit '{}' depends on itself", unit.qualified()).into());
            }
        }

        // Kahn's algorithm: stable order by processing lowest index first.
        let mut in_degree: Vec<usize> = edges.iter().map(BTreeSet::len).collect();
        let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); n];
        for (u, prereqs) in edges.iter().enumerate() {
            for &dep in prereqs {
                dependents[dep].push(u);
            }
        }
        let mut queue: VecDeque<usize> = (0..n).filter(|&u| in_degree[u] == 0).collect();
        let mut topo_order = Vec::with_capacity(n);
        while let Some(u) = queue.pop_front() {
            topo_order.push(u);
            for &v in &dependents[u] {
                in_degree[v] -= 1;
                if in_degree[v] == 0 {
                    queue.push_back(v);
                }
            }
        }
        if topo_order.len() != n {
            let cyclic: Vec<String> = (0..n)
                .filter(|&u| in_degree[u] > 0)
                .map(|u| self.units[u].qualified())
                .collect();
            return Err(anyhow::anyhow!("dependency cycle detected among units: {}", cyclic.join(", ")).into());
        }

        let deps = (0..n)
            .map(|u| UnitDeps {
                after: edges[u].iter().copied().collect(),
                requires: requires[u].iter().copied().collect(),
            })
            .collect();
        Ok(Schedule { topo_order, deps })
    }

    pub fn serialize(&self, w: impl std::io::Write) {
        let json = &mut serde_json::Serializer::new(w);
        let mut erased = <dyn erased_serde::Serializer>::erase(json);
        for rule in self.rules() {
            rule.erased_serialize(&mut erased).expect("failed to serialize");
        }
    }

    pub fn add_rule(&mut self, rule: impl Rule) {
        self.host_rules.push(Box::new(rule));
    }

    pub fn merge(&mut self, other: State) {
        let offset = self.host_rules.len();
        self.host_rules.extend(other.host_rules);
        self.hosts.extend(other.hosts);
        for mut unit in other.units {
            unit.rules = (unit.rules.start + offset)..(unit.rules.end + offset);
            self.units.push(unit);
        }
    }

    pub fn hosts(&self) -> Vec<String> {
        self.hosts.iter().map(|h| h.name().to_string()).collect()
    }
}

// static STATE: Mutex<State> = Mutex::new(State::new());

// pub fn add_to_state(rule: impl Rule) {
//     STATE.lock().unwrap().host_rules.push(Box::new(rule));
// }

// pub fn drop_last_rule(identifier: &str) {
//     let Some(rule) = STATE.lock().unwrap().host_rules.pop() else {
//         panic!("No last rule to drop");
//     };
//     let id = rule.identifier();
//     if id != identifier {
//         panic!("Dropped rule {id}, but expected to drop rule {identifier}");
//     }
// }

// extern "C" fn serialize_state_to_stdout() {
//     let state = STATE.lock().unwrap();
//     let mut stdout = std::io::stdout().lock();
//     state.serialize(&mut stdout);
//     stdout.write("\n".as_bytes()).unwrap();
// }

// #[cfg(feature = "atexit")]
// #[ctor::ctor]
// fn register_at_exit() {
//     unsafe {
//         let result = libc::atexit(serialize_state_to_stdout);
//         if result != 0 {
//             panic!("Failed to register cook serialization on atexit");
//         }
//     }
// }
