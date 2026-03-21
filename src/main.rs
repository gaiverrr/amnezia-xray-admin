mod app;
mod config;
mod error;
mod ssh;
mod ui;
mod xray;

use clap::Parser;
use config::{Cli, Config};

fn main() {
    let cli = Cli::parse();

    let mut config = match Config::load() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Warning: failed to load config: {}", e);
            Config::default()
        }
    };

    config.merge_cli(&cli);

    if config.has_connection_info() {
        println!("amnezia-xray-admin v0.1.0");
        println!(
            "Connecting to {}...",
            config
                .ssh_host
                .as_deref()
                .or(config.host.as_deref())
                .unwrap_or("unknown")
        );
    } else {
        println!("amnezia-xray-admin v0.1.0");
        println!("No connection configured. Run setup or use --host / --ssh-host.");
    }
}
