use std::path::Path;

use clap::{Parser, Subcommand, ValueEnum};
use kdl::KdlDocument;
mod command;
use cook::Check;

#[derive(Copy, Clone, ValueEnum)]
pub enum Method {
    Auto,
    Ssh,
    Agent,
}

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[clap(long, env = "COOK_METHOD", global = true, default_value = "auto")]
    method: Method,
    #[clap(long, env = "COOK_ROOT", global = true, default_value = ".")]
    root: String,
    #[clap(long, short = 'H', env = "COOK_HOST", global = true)]
    host: Vec<String>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// does testing things
    Plan(command::Plan),
    Apply(command::Apply),
    Run(command::Run),
    /// Install agent on remote hosts
    Install(command::Install),
    Ssh(command::Ssh),
}

fn main() {
    let mut cli = Cli::parse();

    build_rules(&mut cli);

    match &cli.command {
        Command::Plan(plan) => plan.run(&cli),
        Command::Apply(_) => {}
        Command::Run(run) => run.run(&cli),
        Command::Install(install) => install.run(&cli),
        Command::Ssh(ssh) => {
            tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(async { ssh.run(&cli).await });
        }
    }
}

fn build_rules(cli: &mut Cli) -> Vec<Box<dyn Check + 'static>> {
    let hosts = &mut cli.host;
    let dir = Path::new(&cli.root);
    let mut rules: Vec<u32> = Vec::new();

    for entry in std::fs::read_dir(dir).expect("Failed to read directory") {
        let entry = entry.expect("Failed to read directory entry");
        let path = entry.path();

        if !path.is_file() {
            continue;
        }
        if kdl_path(&path) {
            let contents = std::fs::read_to_string(&path).expect("Failed to read file");
            let doc = KdlDocument::parse(&contents);
            match doc {
                Err(e) => {
                    for d in e.diagnostics {
                        println!("{:?}", d);
                    }
                    break;
                }
                Ok(doc) => {
                    // Process the document here
                    for node in doc.nodes() {
                        match node.name().value() {
                            "host" => {
                                let entry = node.entry(0).unwrap();
                                hosts.push(entry.value().to_string());
                            }
                            _ => {}
                        }
                        println!("{:?}", node);
                    }
                }
            }
        }
    }
    Vec::new()
}

fn kdl_path(path: &Path) -> bool {
    let file_name = path.file_name().unwrap_or_default();
    if file_name == "Cookfile" {
        true
    } else {
        let Some(extension) = path.extension() else {
            return false;
        };
        extension == "kdl"
    }
}
