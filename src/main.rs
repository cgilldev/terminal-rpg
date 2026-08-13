use std::{net::SocketAddr, path::PathBuf, process::ExitCode};

use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use terminal_rpg::{
    app::{AppMode, PlayOptions, ServeOptions, WebOptions},
    game::RunSeed,
    server::{DEFAULT_HOST_KEY, DEFAULT_LISTEN, validate_host_key_path},
    ui::DisplayProfile,
    web::DEFAULT_WEB_LISTEN,
};
use tracing::info;
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(name = "terminal-rpg", version, about)]
struct Cli {
    #[command(subcommand)]
    command: CliCommand,
}

#[derive(Debug, Subcommand)]
enum CliCommand {
    /// Play in the current terminal.
    Play(PlayArgs),
    /// Serve independent game sessions over SSH.
    Serve(ServeArgs),
    /// Serve independent game sessions in a browser terminal viewport.
    Web(WebArgs),
}

#[derive(Debug, Args)]
struct PlayArgs {
    /// Reproduce a run from this seed.
    #[arg(long)]
    seed: Option<u64>,

    #[command(flatten)]
    display: DisplayArgs,

    #[arg(long, help = "Enable development-only godmode toggle")]
    debug_godmode: bool,
}

#[derive(Debug, Args)]
struct ServeArgs {
    /// Address on which the development SSH server will listen.
    #[arg(long, default_value = DEFAULT_LISTEN)]
    listen: SocketAddr,

    /// Ed25519 host-key path; the server will create it when absent.
    #[arg(long, default_value = DEFAULT_HOST_KEY)]
    host_key: PathBuf,

    /// Reproduce the same run for every development test session.
    #[arg(long)]
    seed: Option<u64>,

    #[command(flatten)]
    display: DisplayArgs,

    #[arg(long, help = "Enable development-only godmode toggle")]
    debug_godmode: bool,
}

#[derive(Debug, Args)]
struct WebArgs {
    /// Address for the unauthenticated development web server.
    #[arg(long, default_value = DEFAULT_WEB_LISTEN)]
    listen: SocketAddr,

    /// Reproduce the same run for every development test session.
    #[arg(long)]
    seed: Option<u64>,

    #[command(flatten)]
    display: DisplayArgs,

    #[arg(long, help = "Enable development-only godmode toggle")]
    debug_godmode: bool,
}

#[derive(Debug, Args)]
struct DisplayArgs {
    /// Force strict ASCII glyphs.
    #[arg(long)]
    ascii: bool,

    /// Disable ANSI color.
    #[arg(long)]
    no_color: bool,
}

impl From<DisplayArgs> for DisplayProfile {
    fn from(value: DisplayArgs) -> Self {
        Self {
            ascii: value.ascii,
            no_color: value.no_color,
        }
    }
}

fn main() -> ExitCode {
    init_diagnostics();
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(error = ?error, "terminal-rpg failed");
            ExitCode::FAILURE
        }
    }
}

fn init_diagnostics() {
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("terminal_rpg=info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .try_init();
}

fn run(cli: Cli) -> Result<()> {
    let mode = match cli.command {
        CliCommand::Play(args) => AppMode::Play(PlayOptions {
            seed: args.seed.map(RunSeed),
            display: args.display.into(),
            debug_godmode: args.debug_godmode,
        }),
        CliCommand::Serve(args) => {
            validate_host_key_path(&args.host_key).context("invalid server configuration")?;
            AppMode::Serve(ServeOptions {
                listen: args.listen,
                host_key: args.host_key,
                seed: args.seed.map(RunSeed),
                display: args.display.into(),
                debug_godmode: args.debug_godmode,
            })
        }
        CliCommand::Web(args) => AppMode::Web(WebOptions {
            listen: args.listen,
            seed: args.seed.map(RunSeed),
            display: args.display.into(),
            debug_godmode: args.debug_godmode,
        }),
    };

    info!(?mode, "mode configured");
    match mode {
        AppMode::Play(options) => {
            terminal_rpg::ui::run_local(options.seed, options.display, options.debug_godmode)
                .context("run local terminal game")
        }
        AppMode::Serve(options) => tokio::runtime::Runtime::new()
            .context("create Tokio runtime")?
            .block_on(terminal_rpg::server::serve(options)),
        AppMode::Web(options) => tokio::runtime::Runtime::new()
            .context("create Tokio runtime")?
            .block_on(terminal_rpg::web::serve(options)),
    }
}
