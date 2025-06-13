use clap::{Parser, Subcommand};
mod command;

#[derive(Parser)]
#[command(version, about)]
struct Cli {
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
}

fn main() {
    let cli = Cli::parse();
    match &cli.command {
        Command::Plan(_) => {}
        Command::Apply(_) => {}
        Command::Run(run) => run.run(&cli),
    }
}
