use clap::Parser;
use grimoire::cli::{self, Cli};

fn main() {
    let cli = Cli::parse();
    let suppress_nag = cli::is_self_update_command(&cli);
    let result = cli::run(cli);
    if !suppress_nag {
        grimoire::upgrade::maybe_nag();
    }
    if let Err(err) = result {
        eprintln!("error: {err:#}");
        std::process::exit(1);
    }
}
