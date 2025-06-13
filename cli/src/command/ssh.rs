use clap::Parser;
use openssh::{KnownHosts, Session};

use crate::Cli;

#[derive(Parser)]
pub struct Ssh {}

impl Ssh {
    pub async fn run(&self, cli: &Cli) {
        let host = cli.host.first().unwrap();

        std::process::Command::new("ssh")
            .arg(host)
            .status()
            .unwrap();
    }
}
