use clap::Parser;
use grimoire::cli::{self, Cli};

fn main() {
    let cli = Cli::parse();
    if let Err(err) = cli::run(cli) {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}
