use clap::Parser;
use cook::State;

use crate::{
    Cli, Method,
    command::{check_cook_agent, connect_ssh, run_over_ssh},
};

#[derive(Parser)]
pub struct Up {}

impl Up {
    pub async fn run(&self, cli: &Cli, state: State) {
        if cli.host.is_empty() {
            panic!("No host specified");
        }
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
