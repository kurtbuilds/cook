use clap::Parser;
use openssh::{KnownHosts, Session};

use crate::Cli;

#[derive(Parser)]
pub struct Install {
    /// Installation directory on remote hosts
    #[clap(long, short, default_value = "/usr/local/bin")]
    install_dir: String,

    /// SSH user for remote connection
    #[clap(long, short)]
    user: Option<String>,

    /// SSH private key file
    #[clap(long, short)]
    key: Option<String>,
}

impl Install {
    pub async fn run(&self, cli: &Cli) {
        let hosts = &cli.host;

        for host in hosts {
            let session = Session::connect_mux(host, KnownHosts::Strict)
                .await
                .expect("Failed to connect to host");
            let output = session
                // .command("sh -c '")
                .command("sh")
                .arg("-c")
                .arg("echo $(uname -s) $(uname -m) $(ls /lib/$(uname -m)-linux-gnu/libc.so.6)")
                .output()
                .await
                .expect("Failed to get triple data");
            let triple_output =
                String::from_utf8(output.stdout).expect("server output was not valid UTF-8");
            let mut parts = triple_output.trim_end().split(" ");
            let os = parts.next().expect("Invalid triple output");
            let arch = parts.next().expect("Invalid triple output");
            let libc = parts.next().unwrap_or_default();
            let os = match os {
                "Linux" => "unknown-linux",
                "Darwin" => "apple-darwin",
                _ => panic!("Unsupported OS"),
            };
            let arch = match arch {
                "x86_64" => "x86_64",
                "aarch64" | "arm64" => "aarch64",
                _ => panic!("Unsupported architecture"),
            };
            let mut platform = format!("{arch}-{os}");
            if os == "unknown-linux" {
                if libc.is_empty() {
                    platform.push_str("-musl");
                } else {
                    platform.push_str("-gnu");
                }
            }
            println!("{}", platform);

            // based on the platform, check
            // (a) github releases
            // (b) local build
            // (c) build it from scratch

            // println!("Would install agent on host: {}", host);
            // println!("  Install directory: {}", self.install_dir);
            // if let Some(user) = &self.user {
            //     println!("  SSH user: {}", user);
            // }
            // if let Some(key) = &self.key {
            //     println!("  SSH key: {}", key);
            // }
        }

        // Stub implementation - actual installation logic would go here:
        // 1. Check if agent binary exists locally
        // 2. Establish SSH connection to remote host
        // 3. Transfer agent binary to remote host
        // 4. Set appropriate permissions
        // 5. Optionally install as a service/daemon
        // 6. Verify installation

        println!("Install command completed (stub implementation)");
    }
}
