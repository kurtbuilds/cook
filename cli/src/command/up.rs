use clap::Parser;

#[derive(Parser)]
pub struct Up {}

impl Up {
    pub async fn run(&self) {
        println!("Running up command");
    }
}
