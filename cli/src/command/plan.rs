use std::path::Path;

use clap::Parser;
use kdl::KdlDocument;

use crate::Cli;

#[derive(Parser)]
pub struct Plan {}

impl Plan {
    pub fn run(&self, cli: &Cli) {
        // find any kdl files.

        let dir = Path::new(&cli.root);
        // let current_dir = std::env::current_dir().expect("Failed to get current directory");

        for entry in std::fs::read_dir(dir).expect("Failed to read directory") {
            let entry = entry.expect("Failed to read directory entry");
            let path = entry.path();

            if path.is_file() {
                if !kdl_path(&path) {
                    continue;
                }
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
                            println!("{:?}", node);
                        }
                    }
                }
            }
        }
    }
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
