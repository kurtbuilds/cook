use clap::Parser;
use kdl::KdlDocument;

#[derive(Parser)]
pub struct Plan {}

impl Plan {
    pub fn run() {
        // find any kdl files.
        let current_dir = std::env::current_dir().expect("Failed to get current directory");

        for entry in std::fs::read_dir(current_dir).expect("Failed to read directory") {
            let entry = entry.expect("Failed to read directory entry");
            let path = entry.path();

            if path.is_file() {
                if let Some(extension) = path.extension() {
                    if extension != "kdl" {
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
}
