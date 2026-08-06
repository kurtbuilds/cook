use clap::Parser;
use colored::Colorize;
use cook::State;
use openssh::Session;
use serde::Serialize;
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
#[derive(Clone)]
enum UnitOutcome {
    /// Completed; carries the serialized modification outputs.
    Done(Arc<Vec<String>>),
    /// Not run because a `requires` dependency failed or was skipped.
    Skipped,
    /// A rule in the unit errored.
    Failed(Arc<str>),
}

/// Apply the config to one host, honoring sequencing directives.
///
/// Units run in dependency order. A `requires` dependency that fails causes its
/// dependents to be skipped rather than aborting the whole run.
///
/// Returns `true` if no unit failed.
pub async fn run_over_ssh(cli: &Cli, session: Session, state: &State, host: &str) -> bool {
    let session = Arc::new(session);
    let units = state.units();
    let schedule = state
        .build_schedule()
        .unwrap_or_else(|e| panic!("invalid sequencing in config: {e}"));

    let mut outcomes: Vec<Option<UnitOutcome>> = vec![None; units.len()];
    for &u in &schedule.topo_order {
        let mut skip = false;
        for &dep in &schedule.deps[u].after {
            let outcome = outcomes[dep]
                .as_ref()
                .expect("topological order guarantees dependencies are built first");
            if schedule.deps[u].requires.contains(&dep) && !matches!(outcome, UnitOutcome::Done(_)) {
                skip = true;
            }
        }

        let outcome = if skip {
            UnitOutcome::Skipped
        } else {
            match run_unit_rules(cli, state, session.clone(), units[u].rules.clone()).await {
                Ok(outputs) => UnitOutcome::Done(Arc::new(outputs)),
                Err(e) => UnitOutcome::Failed(Arc::from(format!("unit '{}': {e}", units[u].qualified()))),
            }
        };
        outcomes[u] = Some(outcome);
    }

    let outcomes: Vec<UnitOutcome> = outcomes
        .into_iter()
        .map(|outcome| outcome.expect("all units built"))
        .collect();

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
