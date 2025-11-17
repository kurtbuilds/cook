use std::path::Path;

use clap::{Parser, Subcommand, ValueEnum};
use cook::{Context, State, add_kdl_deserializers_to_context};
use tracing::level_filters::LevelFilter;
mod command;
mod kdl;

#[derive(Copy, Clone, ValueEnum)]
pub enum Method {
    Auto,
    Ssh,
    Agent,
}

#[derive(Copy, Clone, ValueEnum)]
pub enum Format {
    Human,
    Json,
}

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    /// Enable debug mode, print output from commands
    #[clap(short, long, global = true, default_value = "false")]
    verbose: bool,
    /// output format
    #[clap(short, long, env = "COOK_FORMAT", global = true, default_value = "human")]
    format: Format,
    /// path to javascript interpreter if compiling a cook javascript project
    javascript: Option<String>,
    /// path to typescript interpreter if compiling a cook typescript project
    typescript: Option<String>,
    /// path to python interpreter if compiling a cook python project
    python: Option<String>,
    /// when applying changes, you can use ssh, the agent, or auto-select
    ///
    #[clap(long, env = "COOK_METHOD", global = true, default_value = "auto")]
    method: Method,
    /// Where to calculate paths relative to (on the local (orchestrator) file system)
    #[clap(long, env = "COOK_ROOT", global = true, default_value = ".")]
    root: String,
    /// Specify a specific host to operate on
    #[clap(long, short = 'H', env = "COOK_HOST", global = true)]
    host: Vec<String>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    // /// does testing things
    // Plan(command::Plan),
    // Apply(command::Apply),
    // /// Install agent on remote hosts
    Ssh(command::Ssh),
    Install(command::Install),
    Run(command::Run),
    Preview(command::Preview),
    Up(command::Up),
}

fn main() {
    let mut cli = Cli::parse();

    let path = Path::new(&cli.root);
    let state = build_state(path);
    if cli.host.is_empty() {
        cli.host = state.hosts();
    }

    if cli.verbose {
        tracing_subscriber::fmt().with_max_level(LevelFilter::DEBUG).init();
    }

    match &cli.command {
        Command::Install(install) => {
            tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(async { install.run(&cli).await });
        }
        Command::Ssh(ssh) => {
            tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(async { ssh.run(&cli).await });
        }
        Command::Run(run) => {
            tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(async { run.run(&cli).await });
        }
        Command::Preview(_preview) => todo!(),
        Command::Up(up) => {
            tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(async { up.run(&cli, state).await });
        }
    }
}

// #[derive(Default)]
// pub struct State {
//     pub hosts: Vec<String>,
// }

// impl State {
//     pub fn merge(&mut self, other: State) {
//         self.hosts.extend(other.hosts);
//     }
// }

fn build_state(root: &Path) -> State {
    let mut state = State::new();
    for entry in std::fs::read_dir(root).expect("Failed to read directory") {
        let entry = entry.expect("Failed to read directory entry");
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let file_name = path
            .file_name()
            .unwrap_or_default()
            .to_str()
            .expect("file name is not a valid string");
        let extension = path
            .extension()
            .unwrap_or_default()
            .to_str()
            .expect("extension is not a valid string");
        if file_name == "Cookfile" || extension == "kdl" {
            let content = std::fs::read_to_string(&path).expect("Failed to read file");
            let mut cx = Context::new(root);
            add_kdl_deserializers_to_context(&mut cx);
            let s = kdl::parse_kdl(&content, cx);
            state.merge(s);
        } else if file_name == "main.py" {
        } else if file_name == "main.ts" {
        } else if file_name == "main.js" {
        } else if file_name == "Cargo.toml" {
            let Ok(content) = std::fs::read_to_string(&path) else {
                continue;
            };
            if content.contains("[workspace]") || !content.contains("\ncook =") {
                continue;
            }
            // let contezt = Context::new(root, Some(path));
            // let s = kdl::parse_kdl(&content, contezt);
            // state.merge(s);
        }
    }
    state
}
