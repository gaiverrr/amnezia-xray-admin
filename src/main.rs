mod app;
mod backend;
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

    // Create tokio runtime for async SSH/Xray operations
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("Failed to create async runtime: {}", e);
            std::process::exit(1);
        }
    };

    // Initialize terminal
    let mut terminal = match app::init_terminal() {
        Ok(t) => t,
        Err(e) => {
            eprintln!("Failed to initialize terminal: {}", e);
            std::process::exit(1);
        }
    };

    // Create app and run event loop
    let mut application = app::App::with_config(config, runtime.handle().clone());
    let result = app::run(&mut application, &mut terminal);

    // Always restore terminal, even on error
    if let Err(e) = app::restore_terminal(&mut terminal) {
        eprintln!("Failed to restore terminal: {}", e);
    }

    if let Err(e) = result {
        eprintln!("Application error: {}", e);
        std::process::exit(1);
    }
}
