//! Migration commands for importing from other agent frameworks.

use std::path::PathBuf;

/// Source framework to migrate from.
#[derive(Clone, clap::ValueEnum)]
pub enum MigrateSourceArg {
    /// Migrate from OpenClaw
    Openclaw,
    /// Migrate from LangChain
    Langchain,
    /// Migrate from AutoGPT
    Autogpt,
}

/// Arguments for the migrate command.
#[derive(clap::Args)]
pub struct MigrateArgs {
    /// Source framework to migrate from.
    #[arg(long, value_enum)]
    pub from: MigrateSourceArg,
    /// Path to the source workspace (auto-detected if not set).
    #[arg(long)]
    pub source_dir: Option<PathBuf>,
    /// Dry run — show what would be imported without making changes.
    #[arg(long)]
    pub dry_run: bool,
}

/// Migrate from another agent framework to Skipper.
pub fn cmd_migrate(args: MigrateArgs) {
    let source = match args.from {
        MigrateSourceArg::Openclaw => skipper_migrate::MigrateSource::OpenClaw,
        MigrateSourceArg::Langchain => skipper_migrate::MigrateSource::LangChain,
        MigrateSourceArg::Autogpt => skipper_migrate::MigrateSource::AutoGpt,
    };

    let source_dir = args.source_dir.unwrap_or_else(|| {
        let home = dirs::home_dir().unwrap_or_else(|| {
            eprintln!("Error: Could not determine home directory");
            std::process::exit(1);
        });
        match source {
            skipper_migrate::MigrateSource::OpenClaw => home.join(".openclaw"),
            skipper_migrate::MigrateSource::LangChain => home.join(".langchain"),
            skipper_migrate::MigrateSource::AutoGpt => home.join("Auto-GPT"),
        }
    });

    let target_dir = dirs::home_dir()
        .unwrap_or_else(|| {
            eprintln!("Error: Could not determine home directory");
            std::process::exit(1);
        })
        .join(".skipper");

    println!("Migrating from {} ({})...", source, source_dir.display());
    if args.dry_run {
        println!("  (dry run — no changes will be made)\n");
    }

    let options = skipper_migrate::MigrateOptions {
        source,
        source_dir,
        target_dir,
        dry_run: args.dry_run,
    };

    match skipper_migrate::run_migration(&options) {
        Ok(report) => {
            report.print_summary();

            // Save migration report
            if !args.dry_run {
                let report_path = options.target_dir.join("migration_report.md");
                if let Err(e) = std::fs::write(&report_path, report.to_markdown()) {
                    eprintln!("Warning: Could not save migration report: {e}");
                } else {
                    println!("\n  Report saved to: {}", report_path.display());
                }
            }
        }
        Err(e) => {
            eprintln!("Migration failed: {e}");
            std::process::exit(1);
        }
    }
}
