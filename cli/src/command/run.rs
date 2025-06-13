use clap::Parser;
use cook::cp;

use crate::Cli;

#[derive(Parser)]
pub struct Run {
    command: Vec<String>,
}

impl Run {
    pub fn run(&self, cli: &Cli) {
        dbg!(&cli.host);
        dbg!(&self.command);
        // Implement the logic for running the command here
        for host in &cli.host {
            // run the command
            let mut args = self.command.iter();
            let command = args.next().unwrap();
            match command.as_ref() {
                "cp" => {
                    let src = args.next().unwrap();
                    let dst = args.next().unwrap();
                    cp(src, dst)
                }
            }
        }
    }
}
