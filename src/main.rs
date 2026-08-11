use std::io::{self, Write};
use std::process::ExitCode;

use clap::Parser;
use repomonk::app::{App, AppConfig};
use repomonk::cli::Cli;
use repomonk::config::{default_config_path, load as load_user_config};
use repomonk::store::{purge, DataPaths};
use repomonk::Error;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(Error::PurgeCancelled) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::from(err.exit_code() as u8)
        }
    }
}

fn run() -> repomonk::Result<()> {
    let cli = Cli::parse();

    if let Some(repomonk::cli::Commands::Version) = cli.command {
        println!("repomonk {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    let paths = resolve_paths(&cli)?;

    if cli.purge {
        return run_purge(&paths, cli.yes);
    }

    let config_path = default_config_path()?;
    let user = load_user_config(&config_path)?;
    let cfg = AppConfig {
        cache_dir: paths.cache_dir.clone(),
        db_path: paths.db_path.clone(),
        refresh: cli.refresh,
        no_fx: cli.no_fx,
        user,
        config_path,
    };

    let Some(target) = cli.target else {
        let mut app = App::home(&cfg)?;
        return app.run();
    };

    let mut app = App::open(&target, &cfg)?;
    app.run()
}

fn resolve_paths(cli: &Cli) -> repomonk::Result<DataPaths> {
    if let (Some(cache), Some(data)) = (&cli.cache_dir, &cli.data_dir) {
        return Ok(DataPaths::from_roots(cache.clone(), data.clone()));
    }
    let mut paths = DataPaths::default_user()?;
    if let Some(cache) = &cli.cache_dir {
        paths.cache_dir = cache.clone();
    }
    if let Some(data) = &cli.data_dir {
        paths.data_dir = data.clone();
        paths.db_path = data.join("repomonk.db");
    }
    Ok(paths)
}

fn run_purge(paths: &DataPaths, yes: bool) -> repomonk::Result<()> {
    println!("This will delete repomonk-managed data:");
    println!("  cache: {}", paths.cache_dir.display());
    println!("  data:  {}", paths.data_dir.display());
    if !yes {
        print!("Type 'yes' to confirm: ");
        io::stdout().flush()?;
        let mut line = String::new();
        io::stdin().read_line(&mut line)?;
        if line.trim() != "yes" {
            println!("cancelled");
            return Err(Error::PurgeCancelled);
        }
    }
    purge(paths)?;
    println!("purged");
    Ok(())
}
