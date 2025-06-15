use clap::Parser;

use crate::Cli;

#[derive(Parser)]
pub struct Ssh {
    command: Vec<String>,
}

impl Ssh {
    pub async fn run(&self, cli: &Cli) {
        if self.command.is_empty() {
            let host = cli.host.first().unwrap();
            std::process::Command::new("ssh")
                .arg(host)
                .status()
                .unwrap();
        } else {
            if cli.host.is_empty() {
                panic!("You must provide a host");
            }
            for host in cli.host.iter() {
                std::process::Command::new("ssh")
                    .arg(host)
                    .args(self.command.iter())
                    .status()
                    .unwrap();
            }
        }
    }
}
