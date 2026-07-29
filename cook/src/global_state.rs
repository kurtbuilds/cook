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
    /// Name other units reference in `after`/`before`/`requires`. Defaults to
    /// the identifier of the unit's first rule.
    pub name: String,
    /// Range of [`State::rules`] indices owned by this unit.
    pub rules: Range<usize>,
    pub after: Vec<String>,
    pub before: Vec<String>,
    pub requires: Vec<String>,
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
        let name = sequencing
            .name
            .unwrap_or_else(|| self.host_rules[rules.start].identifier().to_string());
        self.units.push(Unit {
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
    /// Errors on duplicate unit names, references to unknown units, self
    /// dependencies, and dependency cycles.
    pub fn build_schedule(&self) -> Result<Schedule, crate::Error> {
        let n = self.units.len();

        let mut index: BTreeMap<&str, usize> = BTreeMap::new();
        for (i, unit) in self.units.iter().enumerate() {
            if index.insert(unit.name.as_str(), i).is_some() {
                return Err(anyhow::anyhow!("duplicate unit name '{}'", unit.name).into());
            }
        }
        let resolve = |referrer: &str, name: &str| -> Result<usize, crate::Error> {
            index
                .get(name)
                .copied()
                .ok_or_else(|| anyhow::anyhow!("unit '{referrer}' references unknown unit '{name}'").into())
        };

        // edges[u] = units that must run before u. requires[u] ⊆ edges[u].
        let mut edges: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); n];
        let mut requires: Vec<BTreeSet<usize>> = vec![BTreeSet::new(); n];
        for (u, unit) in self.units.iter().enumerate() {
            for name in &unit.after {
                edges[u].insert(resolve(&unit.name, name)?);
            }
            for name in &unit.requires {
                let dep = resolve(&unit.name, name)?;
                edges[u].insert(dep);
                requires[u].insert(dep);
            }
            for name in &unit.before {
                // `u before x` means x runs after u.
                let target = resolve(&unit.name, name)?;
                edges[target].insert(u);
            }
        }
        for (u, unit) in self.units.iter().enumerate() {
            if edges[u].contains(&u) {
                return Err(anyhow::anyhow!("unit '{}' depends on itself", unit.name).into());
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
            let cyclic: Vec<&str> = (0..n)
                .filter(|&u| in_degree[u] > 0)
                .map(|u| self.units[u].name.as_str())
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
