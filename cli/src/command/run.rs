use clap::Parser;
use colored::Colorize;
use cook::State;
use futures::future::{FutureExt, Shared, join_all};
use openssh::Session;
use serde::Serialize;
use std::fmt::Display;
use std::future::Future;
use std::ops::Range;
use std::pin::Pin;
use std::sync::Arc;
use tracing::debug;

use crate::{Cli, Context, Format, Method, kdl::parse_kdl};

#[derive(Parser)]
pub struct Run {
    command: Vec<String>,
}

pub async fn connect_ssh(host: &str) -> Session {
    Session::connect_mux(host, openssh::KnownHosts::Strict)
        .await
        .expect("Failed to connect to host")
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
        let context = Context::new(&cli.root);
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
/// Used when output is produced from concurrent tasks: each task serializes into
/// its own buffer so writes to stdout don't interleave.
pub fn serialize_structured<T: erased_serde::Serialize + ?Sized>(format: Format, data: &T) -> String {
    let _ = format;
    let mut buf = Vec::new();
    let mut serializer = serde_json::Serializer::new(&mut buf);
    erased_serde::serialize(data, &mut serializer).expect("Failed to serialize data");
    String::from_utf8(buf).expect("serialized output was not valid utf8")
}

/// The result of running one unit, shared across its dependents.
///
/// `Arc`-wrapped payloads keep cloning cheap, which matters because dependents
/// each hold a clone of their dependencies' [`Shared`] futures.
#[derive(Clone)]
enum UnitOutcome {
    /// Completed; carries the serialized modification outputs.
    Done(Arc<Vec<String>>),
    /// Not run because a `requires` dependency failed or was skipped.
    Skipped,
    /// A rule in the unit errored.
    Failed(Arc<str>),
}

type UnitFut<'a> = Shared<Pin<Box<dyn Future<Output = UnitOutcome> + 'a>>>;

/// Apply the config to one host, honoring sequencing directives.
///
/// Units run as soon as their dependencies complete — independent units run
/// fully concurrently, multiplexed over the single SSH connection, while
/// `after`/`before`/`requires` edges are respected. A `requires` dependency that
/// fails causes its dependents to be skipped rather than aborting the whole run.
///
/// Returns `true` if no unit failed.
pub async fn run_over_ssh(cli: &Cli, session: Session, state: &State, host: &str) -> bool {
    let session = Arc::new(session);
    let units = state.units();
    let schedule = state
        .build_schedule()
        .unwrap_or_else(|e| panic!("invalid sequencing in config: {e}"));

    // Build a Shared future per unit in dependency order, so each unit can
    // capture clones of the futures it must wait for.
    let mut futs: Vec<Option<UnitFut>> = (0..units.len()).map(|_| None).collect();
    for &u in &schedule.topo_order {
        let deps: Vec<(bool, UnitFut)> = schedule.deps[u]
            .after
            .iter()
            .map(|&d| {
                let is_required = schedule.deps[u].requires.contains(&d);
                let dep = futs[d]
                    .clone()
                    .expect("topological order guarantees dependencies are built first");
                (is_required, dep)
            })
            .collect();

        let range = units[u].rules.clone();
        let name = units[u].name.clone();

        let session = session.clone();
        let fut = async move {
            // Wait for ordering deps; skip if any required dep didn't complete.
            let mut skip = false;
            for (is_required, dep) in deps {
                let outcome = dep.await;
                if is_required && !matches!(outcome, UnitOutcome::Done(_)) {
                    skip = true;
                }
            }
            if skip {
                return UnitOutcome::Skipped;
            }
            match run_unit_rules(cli, state, session, range).await {
                Ok(outputs) => UnitOutcome::Done(Arc::new(outputs)),
                Err(e) => UnitOutcome::Failed(Arc::from(format!("unit '{name}': {e}"))),
            }
        };
        let fut: Pin<Box<dyn Future<Output = UnitOutcome> + '_>> = Box::pin(fut);
        futs[u] = Some(fut.shared());
    }

    // Drive every unit. Dependencies are awaited inside each future; `Shared`
    // ensures each unit's work runs at most once.
    let outcomes = join_all(futs.iter().map(|f| f.clone().expect("all units built"))).await;

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
                eprintln!("{skipped} {host}: unit '{}' (required dependency did not complete)", units[u].name);
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

/// Run all rules in a unit concurrently: each rule checks itself, then applies
/// its modifications concurrently. Returns the serialized outputs of every
/// modification applied, or the first error encountered.
async fn run_unit_rules(
    cli: &Cli,
    state: &State,
    session: Arc<Session>,
    range: Range<usize>,
) -> Result<Vec<String>, cook::Error> {
    let rules = state.rules();
    let rule_tasks = range.map(|i| {
        let session = session.clone();
        let rule = &rules[i];
        async move {
            debug!(rule_id = rule.identifier(), "Checking rule");
            let rule = rule
                .downcast_ssh()
                .ok_or_else(|| cook::Error::from("rule cannot run over ssh"))?;
            let modifications = rule.check_ssh(&session).await?;

            let mod_tasks = modifications.into_iter().map(|modification| {
                let session = session.clone();
                async move {
                    let m = modification
                        .downcast_ssh()
                        .ok_or_else(|| cook::Error::from("modification cannot be applied over ssh"))?;
                    m.apply_ssh(session.clone()).await?;
                    let ser: &dyn erased_serde::Serialize = modification.as_ref();
                    Ok::<String, cook::Error>(serialize_structured(cli.format, ser))
                }
            });
            join_all(mod_tasks).await.into_iter().collect::<Result<Vec<_>, _>>()
        }
    });

    let per_rule = join_all(rule_tasks).await;
    let mut outputs = Vec::new();
    for rule_outputs in per_rule {
        outputs.extend(rule_outputs?);
    }
    Ok(outputs)
}
