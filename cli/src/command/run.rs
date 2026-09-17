use clap::Parser;
use colored::Colorize;
use cook::State;
use cook::ssh::Session;
use futures::{FutureExt, StreamExt, future::LocalBoxFuture, stream::FuturesUnordered};
use serde::Serialize;
use std::collections::{BTreeSet, VecDeque};
use std::fmt::Display;
use std::ops::Range;
use std::sync::Arc;
use tracing::debug;

use crate::{Cli, Context, Format, Method, kdl::parse_kdl};

#[derive(Parser)]
pub struct Run {
    command: Vec<String>,
}

pub async fn connect_ssh(host: &str) -> Session {
    Session::connect(host).await.expect("Failed to connect to host")
}

pub async fn check_cook_agent(session: &Session) -> Option<String> {
    let output = session
        .command("sh")
        .arg("-c")
        .arg("PATH=/usr/local/bin:/usr/bin:/opt/cook:$HOME/.cargo/bin: which cook")
        .output()
        .await
        .expect("failed to check for cook")
        .stdout;
    (!output.is_empty()).then(|| String::from_utf8(output).expect("invalid utf8"))
}

impl Run {
    pub async fn run(&self, cli: &Cli) {
        if cli.host.is_empty() {
            panic!("No host specified");
        }
        if self.command.is_empty() {
            panic!("No command to run");
        }
        let command = self.command.join(" ");
        let mut context = Context::new(&cli.root);
        cook::add_kdl_deserializers_to_context(&mut context);
        let state = parse_kdl(&command, context);

        match cli.method {
            Method::Agent => {
                for host in &cli.host {
                    let session = connect_ssh(host).await;
                    let Some(_bin) = check_cook_agent(&session).await else {
                        panic!("Agent was not found on host: {}", host);
                    };
                    todo!()
                }
            }
            Method::Ssh => {
                let mut ok = true;
                for host in &cli.host {
                    let session = connect_ssh(host).await;
                    ok &= run_over_ssh(cli, session, &state, host).await;
                }
                if !ok {
                    std::process::exit(1);
                }
            }
            Method::Auto => {
                let mut ok = true;
                for host in &cli.host {
                    let session = connect_ssh(host).await;
                    let bin = check_cook_agent(&session).await;
                    if let Some(_bin) = bin {
                        // run via agent
                        // /
                    } else {
                        ok &= run_over_ssh(cli, session, &state, host).await;
                    }
                }
                if !ok {
                    std::process::exit(1);
                }
            }
        }
    }
}

#[derive(Serialize)]
pub struct HostComplete {
    host: String,
    completed: bool,
    modifications: usize,
}

impl Display for HostComplete {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let success = "[success]".green();
        let host = &self.host;
        let modifications = self.modifications;
        write!(f, "{success} {host}: {modifications} modifications applied")
    }
}

pub fn structured_output<T: erased_serde::Serialize + ?Sized>(format: Format, data: &T) {
    print!("{}", serialize_structured(format, data));
}

/// Serialize `data` to a string using the same encoding as [`structured_output`].
///
/// Used to serialize modification output before it is emitted.
pub fn serialize_structured<T: erased_serde::Serialize + ?Sized>(format: Format, data: &T) -> String {
    let _ = format;
    let mut buf = Vec::new();
    let mut serializer = serde_json::Serializer::new(&mut buf);
    erased_serde::serialize(data, &mut serializer).expect("Failed to serialize data");
    String::from_utf8(buf).expect("serialized output was not valid utf8")
}

/// The result of running one unit.
#[derive(Clone, Debug)]
enum UnitOutcome {
    /// Completed; carries the serialized modification outputs.
    Done(Arc<Vec<String>>),
    /// Not run because a `requires` dependency failed or was skipped.
    Skipped,
    /// A rule in the unit errored.
    Failed(Arc<str>),
}

type UnitTask<'a> = LocalBoxFuture<'a, Vec<(usize, UnitOutcome)>>;

/// Apply the config to one host, honoring sequencing directives.
///
/// Units run as soon as their ordering dependencies have completed. Independent
/// units can run concurrently over the shared SSH session. A `requires`
/// dependency that fails causes its dependents to be skipped rather than
/// aborting the whole run.
///
/// Returns `true` if no unit failed.
pub async fn run_over_ssh(cli: &Cli, session: Session, state: &State, host: &str) -> bool {
    let session = Arc::new(session);
    let units = state.units();
    let schedule = state
        .build_schedule()
        .unwrap_or_else(|e| panic!("invalid sequencing in config: {e}"));

    let package_units: Vec<Option<Vec<String>>> = units
        .iter()
        .map(|unit| package_names_in_unit(state, unit.rules.clone()))
        .collect();

    let outcomes = run_scheduled_units(&schedule, |ready| {
        let mut package_batch = Vec::new();
        let mut tasks: Vec<UnitTask<'_>> = Vec::new();

        for u in ready {
            if let Some(packages) = &package_units[u] {
                package_batch.push(PackageUnit {
                    index: u,
                    qualified: units[u].qualified(),
                    packages: packages.clone(),
                });
                continue;
            }

            let session = session.clone();
            let range = units[u].rules.clone();
            let qualified = units[u].qualified();
            tasks.push(
                async move {
                    debug!(unit = %qualified, "Starting unit");
                    let outcome = match run_unit_rules(cli, state, session, range).await {
                        Ok(outputs) => UnitOutcome::Done(Arc::new(outputs)),
                        Err(e) => UnitOutcome::Failed(Arc::from(format!("unit '{qualified}': {e}"))),
                    };
                    vec![(u, outcome)]
                }
                .boxed_local(),
            );
        }

        if !package_batch.is_empty() {
            let session = session.clone();
            tasks.push(async move { run_package_units(cli, session, package_batch).await }.boxed_local());
        }

        tasks
    })
    .await;

    let mut count = 0;
    let mut ok = true;
    for (u, outcome) in outcomes.into_iter().enumerate() {
        match outcome {
            UnitOutcome::Done(outputs) => {
                for output in outputs.iter() {
                    count += 1;
                    print!("{output}");
                }
            }
            UnitOutcome::Skipped => {
                let skipped = "[skipped]".yellow();
                eprintln!(
                    "{skipped} {host}: unit '{}' (required dependency did not complete)",
                    units[u].qualified()
                );
            }
            UnitOutcome::Failed(msg) => {
                ok = false;
                let error = "[error]".red();
                eprintln!("{error} {host}: {msg}");
            }
        }
    }

    if ok && count == 0 {
        let success = "[success]".green();
        eprintln!("{success} {host}: No modifications to run");
    } else if ok {
        let output = HostComplete {
            host: host.to_string(),
            completed: true,
            modifications: count,
        };
        structured_output(cli.format, &output);
    }
    ok
}

/// Run units as a dependency graph. Independent units are polled together; a
/// dependent becomes runnable once every `after` dependency has completed.
async fn run_scheduled_units<'a, F>(schedule: &cook::Schedule, mut run_ready_units: F) -> Vec<UnitOutcome>
where
    F: FnMut(Vec<usize>) -> Vec<UnitTask<'a>>,
{
    let n = schedule.deps.len();
    let mut remaining_after: Vec<usize> = schedule.deps.iter().map(|deps| deps.after.len()).collect();
    let mut dependents: Vec<Vec<usize>> = vec![Vec::new(); n];
    for (u, deps) in schedule.deps.iter().enumerate() {
        for &dep in &deps.after {
            dependents[dep].push(u);
        }
    }

    let mut ready: VecDeque<usize> = schedule
        .topo_order
        .iter()
        .copied()
        .filter(|&u| remaining_after[u] == 0)
        .collect();
    let mut running = FuturesUnordered::new();
    let mut outcomes: Vec<Option<UnitOutcome>> = vec![None; n];
    let mut finished = 0;

    while finished < n {
        let mut runnable = Vec::new();
        while let Some(u) = ready.pop_front() {
            let skip = schedule.deps[u].requires.iter().any(|&dep| {
                !matches!(
                    outcomes[dep].as_ref().expect("ready unit has completed dependencies"),
                    UnitOutcome::Done(_)
                )
            });

            if skip {
                outcomes[u] = Some(UnitOutcome::Skipped);
                finished += 1;
                for &dependent in &dependents[u] {
                    remaining_after[dependent] -= 1;
                    if remaining_after[dependent] == 0 {
                        ready.push_back(dependent);
                    }
                }
                continue;
            }

            runnable.push(u);
        }

        if !runnable.is_empty() {
            for task in run_ready_units(runnable) {
                running.push(task);
            }
        }

        if finished == n {
            break;
        }

        let Some(completed) = running.next().await else {
            panic!("scheduler stalled with unfinished units");
        };
        for (u, outcome) in completed {
            outcomes[u] = Some(outcome);
            finished += 1;
            for &dependent in &dependents[u] {
                remaining_after[dependent] -= 1;
                if remaining_after[dependent] == 0 {
                    ready.push_back(dependent);
                }
            }
        }
    }

    outcomes
        .into_iter()
        .map(|outcome| outcome.expect("all units built"))
        .collect()
}

#[derive(Debug)]
struct PackageUnit {
    index: usize,
    qualified: String,
    packages: Vec<String>,
}

#[derive(Serialize)]
enum PackageOutput {
    AddPackage(PackageOutputSpec),
}

#[derive(Serialize)]
struct PackageOutputSpec {
    name: String,
}

fn package_names_in_unit(state: &State, range: Range<usize>) -> Option<Vec<String>> {
    let mut packages = Vec::new();
    for i in range {
        let rule = &state.rules()[i];
        if rule.kind() != "package" {
            return None;
        }
        packages.push(rule.identifier().to_string());
    }
    (!packages.is_empty()).then_some(packages)
}

async fn run_package_units(cli: &Cli, session: Arc<Session>, units: Vec<PackageUnit>) -> Vec<(usize, UnitOutcome)> {
    let mut all_packages = BTreeSet::new();
    for unit in &units {
        for package in &unit.packages {
            all_packages.insert(package.clone());
        }
    }
    let all_packages: Vec<String> = all_packages.into_iter().collect();
    debug!(packages = ?all_packages, "Checking package batch");

    let missing = match missing_packages_ssh(&session, &all_packages).await {
        Ok(missing) => missing,
        Err(e) => return package_batch_failed(units, format!("package batch check failed: {e}")),
    };

    if !missing.is_empty()
        && let Err(e) = install_packages_ssh(session, &missing).await
    {
        return package_batch_failed(units, format!("package batch install failed: {e}"));
    }

    units
        .into_iter()
        .map(|unit| {
            let outputs = unit
                .packages
                .into_iter()
                .filter(|package| missing.contains(package))
                .map(|name| serialize_structured(cli.format, &PackageOutput::AddPackage(PackageOutputSpec { name })))
                .collect();
            (unit.index, UnitOutcome::Done(Arc::new(outputs)))
        })
        .collect()
}

fn package_batch_failed(units: Vec<PackageUnit>, message: String) -> Vec<(usize, UnitOutcome)> {
    units
        .into_iter()
        .map(|unit| {
            (
                unit.index,
                UnitOutcome::Failed(Arc::from(format!("unit '{}': {message}", unit.qualified))),
            )
        })
        .collect()
}

async fn missing_packages_ssh(session: &Session, packages: &[String]) -> Result<BTreeSet<String>, cook::Error> {
    if packages.is_empty() {
        return Ok(BTreeSet::new());
    }

    let mut command = session.command("dpkg-query");
    command
        .arg("-W")
        .arg("-f=${binary:Package}\\n")
        .arg("--")
        .args(packages);
    let output = command.output().await?;
    let stdout = String::from_utf8(output.stdout)?;
    let installed: BTreeSet<&str> = stdout.lines().collect();

    Ok(packages
        .iter()
        .filter(|package| !installed.contains(package.as_str()))
        .cloned()
        .collect())
}

async fn install_packages_ssh(session: Arc<Session>, packages: &BTreeSet<String>) -> Result<(), cook::Error> {
    let mut command = session.command("apt");
    command.arg("install").arg("-y").args(packages);
    let status = command.output().await?.status;
    if !status.success() {
        return Err(format!(
            "apt install failed for {}",
            packages.iter().cloned().collect::<Vec<_>>().join(", ")
        )
        .into());
    }
    Ok(())
}

/// Run all rules in a unit in order. Each rule checks itself, then applies its
/// modifications in order. Returns the serialized outputs of every modification
/// applied, or the first error encountered.
async fn run_unit_rules(
    cli: &Cli,
    state: &State,
    session: Arc<Session>,
    range: Range<usize>,
) -> Result<Vec<String>, cook::Error> {
    let rules = state.rules();
    let mut outputs = Vec::new();
    for i in range {
        let rule = &rules[i];
        debug!(rule_id = rule.identifier(), "Checking rule");
        let rule = rule
            .downcast_ssh()
            .ok_or_else(|| cook::Error::from("rule cannot run over ssh"))?;
        let modifications = rule.check_ssh(&session).await?;

        for modification in modifications {
            let m = modification
                .downcast_ssh()
                .ok_or_else(|| cook::Error::from("modification cannot be applied over ssh"))?;
            m.apply_ssh(session.clone()).await?;
            let ser: &dyn erased_serde::Serialize = modification.as_ref();
            outputs.push(serialize_structured(cli.format, ser));
        }
    }
    Ok(outputs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use cook::{Schedule, UnitDeps};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    fn empty_deps() -> UnitDeps {
        UnitDeps {
            after: Vec::new(),
            requires: Vec::new(),
        }
    }

    #[tokio::test]
    async fn independent_units_are_scheduled_concurrently() {
        let schedule = Schedule {
            topo_order: vec![0, 1, 2],
            deps: vec![empty_deps(), empty_deps(), empty_deps()],
        };
        let running = Arc::new(AtomicUsize::new(0));
        let max_running = Arc::new(AtomicUsize::new(0));

        let outcomes = run_scheduled_units(&schedule, |ready| {
            ready
                .into_iter()
                .map(|u| {
                    let running = running.clone();
                    let max_running = max_running.clone();
                    async move {
                        let now = running.fetch_add(1, Ordering::SeqCst) + 1;
                        max_running.fetch_max(now, Ordering::SeqCst);
                        tokio::time::sleep(Duration::from_millis(50)).await;
                        running.fetch_sub(1, Ordering::SeqCst);
                        vec![(u, UnitOutcome::Done(Arc::new(vec![u.to_string()])))]
                    }
                    .boxed_local()
                })
                .collect()
        })
        .await;

        assert!(
            max_running.load(Ordering::SeqCst) > 1,
            "independent units did not overlap"
        );
        assert!(outcomes.iter().all(|outcome| matches!(outcome, UnitOutcome::Done(_))));
    }

    #[tokio::test]
    async fn requires_dependency_failure_skips_dependent_unit() {
        let schedule = Schedule {
            topo_order: vec![0, 1],
            deps: vec![
                empty_deps(),
                UnitDeps {
                    after: vec![0],
                    requires: vec![0],
                },
            ],
        };
        let started = Arc::new(AtomicUsize::new(0));

        let outcomes = run_scheduled_units(&schedule, |ready| {
            ready
                .into_iter()
                .map(|u| {
                    let started = started.clone();
                    async move {
                        started.fetch_add(1, Ordering::SeqCst);
                        let outcome = if u == 0 {
                            UnitOutcome::Failed(Arc::from("failed"))
                        } else {
                            UnitOutcome::Done(Arc::new(Vec::new()))
                        };
                        vec![(u, outcome)]
                    }
                    .boxed_local()
                })
                .collect()
        })
        .await;

        assert_eq!(started.load(Ordering::SeqCst), 1, "skipped unit should not run");
        assert!(matches!(outcomes[0], UnitOutcome::Failed(_)));
        assert!(matches!(outcomes[1], UnitOutcome::Skipped));
    }

    #[tokio::test]
    async fn scheduler_offers_ready_units_together_for_batching() {
        let schedule = Schedule {
            topo_order: vec![0, 1, 2],
            deps: vec![empty_deps(), empty_deps(), empty_deps()],
        };
        let batch_count = Arc::new(AtomicUsize::new(0));

        let outcomes = run_scheduled_units(&schedule, |ready| {
            let batch_count = batch_count.clone();
            vec![
                async move {
                    batch_count.fetch_add(1, Ordering::SeqCst);
                    ready
                        .into_iter()
                        .map(|u| (u, UnitOutcome::Done(Arc::new(Vec::new()))))
                        .collect()
                }
                .boxed_local(),
            ]
        })
        .await;

        assert_eq!(batch_count.load(Ordering::SeqCst), 1);
        assert!(outcomes.iter().all(|outcome| matches!(outcome, UnitOutcome::Done(_))));
    }
}
