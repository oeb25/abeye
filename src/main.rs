use std::io::Read;

use abeye::{generate_ts, Config, Database, InputApi};
use camino::Utf8PathBuf;
use clap::{Parser, Subcommand, ValueEnum};
use color_eyre::Result;
use openapiv3 as oapi;
use tracing_subscriber::{filter::LevelFilter, prelude::*, EnvFilter};

fn main() -> Result<()> {
    color_eyre::install()?;
    if std::env::var_os("RUST_LOG").is_none() {
        std::env::set_var("RUST_LOG", "abeye=info");
    }
    tracing_subscriber::registry()
        .with(
            EnvFilter::builder()
                .with_default_directive(LevelFilter::INFO.into())
                .from_env_lossy(),
        )
        .with(
            tracing_subscriber::fmt::layer()
                .with_target(false)
                .without_time(),
        )
        .with(tracing_subscriber::filter::FilterFn::new(|m| {
            !m.target().contains("salsa")
        }))
        .with(tracing_error::ErrorLayer::default())
        .init();

    run()?;

    Ok(())
}

fn run() -> Result<()> {
    let cli = Cli::parse();

    match &cli.cmd {
        Command::Generate {
            source,
            target,
            output,
            api_prefix,
        } => {
            let api: oapi::OpenAPI = match source {
                Some(s) if s.starts_with("http://") || s.starts_with("https://") => {
                    tracing::info!(url=?s, "fetching schema");
                    reqwest::blocking::get(s)?.json()?
                }
                Some(s) => serde_json::from_str(&std::fs::read_to_string(s)?)?,
                None => {
                    let mut buf = String::new();
                    std::io::stdin().read_to_string(&mut buf)?;
                    serde_json::from_str(&buf)?
                }
            };

            let db = Database::default();

            let api = InputApi::new(
                &db,
                api,
                Config {
                    api_prefix: api_prefix
                        .clone()
                        .map(|prefix| prefix.trim_end_matches('/').into()),
                },
            );

            let output_text = match target {
                Target::TypeScript => generate_ts(&db, api),
            };

            match output {
                Some(output_path) => {
                    tracing::info!(path=?output_path,"writing output");
                    std::fs::write(output_path, output_text)?;
                }
                None => {
                    println!("{output_text}")
                }
            }
        }
    }

    Ok(())
}

#[derive(Parser)]
#[command(author, version, about)]
struct Cli {
    #[clap(subcommand)]
    cmd: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Generate type definitions and client for the given OpenAPI.
    Generate {
        /// Path or URL of the OpenAPI document. If none is provided the
        /// document will be read from STDIN.
        source: Option<String>,
        /// The output format of the generated file.
        #[clap(long, short)]
        target: Target,
        /// The path where the output will be written. If none is provided the
        /// out generated file will be printed to STDOUT.
        #[clap(long, short)]
        output: Option<Utf8PathBuf>,
        /// A common prefix for API endpoints to exclude when determining names
        /// generated methods.
        ///
        /// For example, given "/beta/api" the endpoints will have names as
        /// follows:
        ///
        /// * "/beta/api/autosuggest"            => "autosuggest"
        ///
        /// * "/beta/api/explore/export"         => "exploreExport"
        ///
        /// * "/beta/api/sites/export"           => "sitesExport"
        ///
        /// * "/beta/api/webgraph/host/ingoing"  => "webgraphHostIngoing"
        ///
        /// * "/beta/api/webgraph/host/outgoing" => "webgraphHostOutgoing"
        #[clap(long)]
        api_prefix: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum Target {
    #[value(name = "ts")]
    TypeScript,
}
