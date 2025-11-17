use clap::Parser;
use colored::Colorize;
use cook::State;
use openssh::Session;
use serde::Serialize;
use std::fmt::Display;
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
                for host in &cli.host {
                    let session = connect_ssh(host).await;
                    run_over_ssh(cli, session, &state, host).await;
                }
            }
            Method::Auto => {
                for host in &cli.host {
                    let session = connect_ssh(host).await;
                    let bin = check_cook_agent(&session).await;
                    if let Some(_bin) = bin {
                        // run via agent
                        // /
                    } else {
                        run_over_ssh(cli, session, &state, host).await;
                    }
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
    match format {
        // Format::Human => eprintln!("{}", data),
        Format::Human => {
            let stdio = std::io::stdout();
            let mut serializer = serde_json::Serializer::new(stdio);
            erased_serde::serialize(data, &mut serializer).expect("Failed to serialize data");
        }
        Format::Json => {
            let stdio = std::io::stdout();
            let mut serializer = serde_json::Serializer::new(stdio);
            erased_serde::serialize(data, &mut serializer).expect("Failed to serialize data");
        }
    }
}

pub async fn run_over_ssh(cli: &Cli, session: Session, state: &State, host: &str) {
    let mut count = 0;
    let session = std::sync::Arc::new(session);
    for rule in state.rules() {
        debug!(rule_id = rule.identifier(), "Checking rule");
        let rule = rule.downcast_ssh().expect("Failed to downcast rule");

        let modifications = rule.check_ssh(&*session).await.expect("failed");
        count += modifications.len();
        for modification in modifications {
            let m = modification.downcast_ssh().expect("Cannot apply modification over ssh");
            m.apply_ssh(session.clone()).await.expect("Failed to apply rule");
            let ser: &dyn erased_serde::Serialize = modification.as_ref();
            structured_output(cli.format, ser);
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
