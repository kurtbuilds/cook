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

pub async fn run_over_ssh(cli: &Cli, session: Session, state: &State, host: &str) {
    let session = std::sync::Arc::new(session);

    // Run every rule concurrently on this host, and within each rule apply its
    // modifications concurrently. Commands are multiplexed over the single SSH
    // connection (`connect_mux`) instead of being awaited one after another.
    //
    // We keep each rule's `check` coupled to applying that rule's own
    // modifications, since a check may observe the result of its own changes.
    let rule_tasks = state.rules().iter().map(|rule| {
        let session = session.clone();
        async move {
            debug!(rule_id = rule.identifier(), "Checking rule");
            let rule = rule.downcast_ssh().expect("Failed to downcast rule");
            let modifications = rule.check_ssh(&*session).await.expect("failed");

            let mod_tasks = modifications.into_iter().map(|modification| {
                let session = session.clone();
                async move {
                    let m = modification
                        .downcast_ssh()
                        .expect("Cannot apply modification over ssh");
                    m.apply_ssh(session.clone())
                        .await
                        .expect("Failed to apply rule");
                    let ser: &dyn erased_serde::Serialize = modification.as_ref();
                    serialize_structured(cli.format, ser)
                }
            });
            futures::future::join_all(mod_tasks).await
        }
    });

    let results = futures::future::join_all(rule_tasks).await;

    let mut count = 0;
    for outputs in results {
        for output in outputs {
            count += 1;
            print!("{output}");
        }
    }
    if count == 0 {
        let success = "[success]".green();
        eprintln!("{success} {host}: No modifications to run");
    } else {
        let output = HostComplete {
            host: host.to_string(),
            completed: true,
            modifications: count,
        };
        structured_output(cli.format, &output);
    }
}
