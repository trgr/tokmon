mod pricing;
mod providers;
mod proxy;
mod report;
mod storage;
mod tui;

use anyhow::Result;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "tokmon", version, about = "htop for your AI spend — track tokens, latency, and cost across LLM providers")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Wrap a command and track all LLM API calls it makes
    Wrap {
        /// Command and arguments to run
        #[arg(trailing_var_arg = true, required = true)]
        cmd: Vec<String>,

        /// Session label for easier identification
        #[arg(short, long)]
        label: Option<String>,

        /// Suppress all stderr output during the session (only show summary at end).
        /// Use this when wrapping TUI apps like Claude Code.
        #[arg(short, long)]
        quiet: bool,
    },

    /// Show a live TUI dashboard of sessions and costs
    Status,

    /// Print a cost/usage report
    Report {
        /// Time range: e.g. "today", "7d", "30d", "all"
        #[arg(short, long, default_value = "today")]
        range: String,

        /// Group by: "session", "model", "provider"
        #[arg(short, long, default_value = "session")]
        group_by: String,
    },

    /// Set a daily spending budget alert
    Budget {
        /// Daily budget in USD (e.g. 5.00)
        #[arg(required = true)]
        amount: f64,
    },

    /// Show the last N requests with details
    Log {
        /// Number of recent requests to show
        #[arg(short, long, default_value = "20")]
        count: usize,
    },

    /// Update or show model pricing configuration
    UpdatePrices {
        /// Show current prices instead of updating the config file
        #[arg(long)]
        show: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Wrap { cmd, label, quiet } => {
            proxy::run_wrap(cmd, label, quiet).await?;
        }
        Commands::Status => {
            tui::run_tui().await?;
        }
        Commands::Report { range, group_by } => {
            report::run_report(&range, &group_by)?;
        }
        Commands::Budget { amount } => {
            let db = storage::Database::open()?;
            db.set_budget(amount)?;
            println!("Daily budget set to ${:.2}", amount);
        }
        Commands::Log { count } => {
            report::run_log(count)?;
        }
        Commands::UpdatePrices { show } => {
            if show {
                pricing::show_prices();
            } else {
                pricing::update_prices()?;
            }
        }
    }

    Ok(())
}
