#![deny(unsafe_code)]

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use nimino_data_ops::{
    backup_replica, repair_replica, restore_replica, verify_replica, ObjectRepairRoots, ObjectSpec,
};

#[derive(Parser)]
#[command(
    name = "nimino-data-ops",
    about = "Verify or repair one Nimino replica"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Emit canonical, projection, object, and effect health facts as JSON.
    Verify {
        #[arg(long)]
        store: PathBuf,
        #[arg(long)]
        community: String,
        #[arg(long)]
        object_root: Option<PathBuf>,
        #[arg(long = "object", value_parser = parse_object)]
        objects: Vec<ObjectSpec>,
    },
    /// Repair an explicitly quarantined target from a selected healthy source.
    Repair {
        #[arg(long)]
        source_store: PathBuf,
        #[arg(long)]
        target_store: PathBuf,
        #[arg(long)]
        quarantine_store: PathBuf,
        #[arg(long)]
        community: String,
        #[arg(long)]
        source_object_root: Option<PathBuf>,
        #[arg(long)]
        target_object_root: Option<PathBuf>,
        #[arg(long)]
        object_quarantine_root: Option<PathBuf>,
        #[arg(long = "object", value_parser = parse_object)]
        objects: Vec<ObjectSpec>,
    },
    /// Create a verified, no-clobber cutover backup bundle.
    Backup {
        #[arg(long)]
        store: PathBuf,
        #[arg(long)]
        community: String,
        #[arg(long)]
        object_root: Option<PathBuf>,
        #[arg(long = "object", value_parser = parse_object)]
        objects: Vec<ObjectSpec>,
        #[arg(long)]
        backup_dir: PathBuf,
    },
    /// Restore a verified bundle into new store and object paths.
    Restore {
        #[arg(long)]
        backup_dir: PathBuf,
        #[arg(long)]
        store: PathBuf,
        #[arg(long)]
        community: String,
        #[arg(long)]
        object_root: Option<PathBuf>,
    },
}

fn main() {
    if let Err(error) = run(Cli::parse()) {
        eprintln!("error: {error:#}");
        std::process::exit(2);
    }
}

fn run(cli: Cli) -> Result<()> {
    let value = match cli.command {
        Command::Verify {
            store,
            community,
            object_root,
            objects,
        } => serde_json::to_value(verify_replica(
            &store,
            &community,
            object_root.as_deref(),
            &objects,
        )?)?,
        Command::Repair {
            source_store,
            target_store,
            quarantine_store,
            community,
            source_object_root,
            target_object_root,
            object_quarantine_root,
            objects,
        } => serde_json::to_value(repair_replica(
            &source_store,
            &target_store,
            &quarantine_store,
            &community,
            ObjectRepairRoots {
                source: source_object_root.as_deref(),
                target: target_object_root.as_deref(),
                quarantine: object_quarantine_root.as_deref(),
            },
            &objects,
        )?)?,
        Command::Backup {
            store,
            community,
            object_root,
            objects,
            backup_dir,
        } => serde_json::to_value(backup_replica(
            &store,
            &community,
            object_root.as_deref(),
            &objects,
            &backup_dir,
        )?)?,
        Command::Restore {
            backup_dir,
            store,
            community,
            object_root,
        } => serde_json::to_value(restore_replica(
            &backup_dir,
            &store,
            object_root.as_deref(),
            &community,
        )?)?,
    };
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

fn parse_object(value: &str) -> Result<ObjectSpec, String> {
    let Some((digest, size)) = value.split_once(':') else {
        return Err("object must be DIGEST:SIZE".to_owned());
    };
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err("object digest must be lowercase SHA-256".to_owned());
    }
    let size = size
        .parse::<u64>()
        .map_err(|_| "object size must be an integer".to_owned())?;
    if size == 0 {
        return Err("object size must be positive".to_owned());
    }
    Ok(ObjectSpec {
        digest: digest.to_owned(),
        size,
    })
}
