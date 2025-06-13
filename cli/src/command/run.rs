use std::fs;

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
        for _host in &cli.host {
            // run the command
            let mut args = self.command.iter();
            let command = args.next().unwrap();
            match command.as_str() {
                "cp" => {
                    let root = &cli.root;
                    let root = fs::canonicalize(root).unwrap();
                    let src = args.next().unwrap();
                    let src = root.join(src);
                    let dst = args.next().unwrap();
                    let dst = root.join(dst);
                    let file = cp(src.to_str().unwrap(), dst.to_str().unwrap());
                    dbg!(file);
                }
                _ => {}
            }
        }
    }
}
