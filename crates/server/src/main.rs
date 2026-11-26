//! Backend executable entry point.

use moatless_backend::startup;

/// jemalloc rather than glibc malloc: under glibc the backend's RSS grew in
/// steps after each reconnect wave and was not returned to the kernel.
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().any(|a| a == "--dump-openapi") {
        let spec = startup::build_merged_openapi();
        #[allow(clippy::print_stdout)] // CLI tool output, not server logging
        {
            println!("{}", spec.to_pretty_json()?);
        }
        return Ok(());
    }

    if std::env::args().nth(1).as_deref() == Some("secrets") {
        use clap::Parser;

        #[derive(Parser)]
        #[command(name = "moatless-backend")]
        struct SecretsEntry {
            #[command(subcommand)]
            command: SecretsTop,
        }

        #[derive(clap::Subcommand)]
        enum SecretsTop {
            #[command(subcommand)]
            Secrets(moatless_backend::secret_cli::SecretsCommand),
        }

        let SecretsEntry {
            command: SecretsTop::Secrets(cmd),
        } = SecretsEntry::parse();
        return moatless_backend::secret_cli::run(moatless_backend::secret_cli::SecretsCli {
            command: cmd,
        })
        .await;
    }

    if std::env::args().nth(1).as_deref() == Some("migrate") {
        let restore = std::env::args().nth(2).as_deref() == Some("restore-squashed-history");
        return moatless_backend::app::migrate(restore).await;
    }

    Box::pin(moatless_backend::app::run()).await
}
