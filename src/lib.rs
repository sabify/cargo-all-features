use clap::{error::ErrorKind, Command, Parser};
use std::{env, error, ffi, process};

pub mod cargo_metadata;
pub mod features_finder;
pub mod test_runner;
mod types;

#[derive(Parser)]
#[command(author, version, about = "See https://crates.io/crates/cargo-all-features", long_about = None)]
struct Cli {
    #[arg(
        long,
        default_value_t = 1,
        help = "Split the workspace into n chunks, each chunk containing a roughly equal number of crates"
    )]
    n_chunks: usize,
    #[arg(
        long,
        default_value_t = 0,
        requires = "n_chunks",
        help = "Which chunk to test, indexed at 0"
    )]
    chunk: usize,

    #[arg(
        help = "arguments to pass down to cargo",
        allow_hyphen_values = true,
        trailing_var_arg = true
    )]
    cargo_args: Vec<String>,
}

pub fn run(cargo_command: test_runner::CargoCommand) -> Result<(), Box<dyn error::Error>> {
    let cli = Cli::parse();
    let mut cmd = Command::new("cargo-all-features");

    if cli.chunk >= cli.n_chunks {
        cmd.error(
            ErrorKind::InvalidValue,
            "Must not ask for chunks out of bounds",
        )
        .print()?;
        process::exit(1);
    }

    if cli.n_chunks == 0 {
        cmd.error(ErrorKind::InvalidValue, "--n-chunks must be at least 1")
            .print()?;
        process::exit(1)
    }

    let packages = determine_packages_to_test()?;

    // chunks() takes a chunk size, not a number of chunks
    // we must adjust to deal with the fact that if things are not a perfect multiple,
    // len / n_chunks will end up with an uncounted remainder chunk
    let mut chunk_size = packages.len() / cli.n_chunks;
    if packages.len() % cli.n_chunks != 0 {
        chunk_size += 1;
    }

    let chunk = if let Some(chunk) = packages.chunks(chunk_size).nth(cli.chunk) {
        chunk
    } else {
        println!("Chunk is empty (did you ask for more chunks than there are packages?");
        return Ok(());
    };
    if cli.n_chunks != 1 {
        let packages: String = chunk.iter().map(|p| [&p.name, ","]).flatten().collect();
        let packages = packages.trim_end_matches(',');
        println!(
            "Running on chunk {} out of {} ({chunk_size} packages: {packages})",
            cli.chunk, cli.n_chunks
        );
    }

    for package in chunk {
        let outcome = test_all_features_for_package(&package, cargo_command, &cli.cargo_args)?;

        if let TestOutcome::Fail(exit_status) = outcome {
            process::exit(exit_status.code().unwrap());
        }
    }

    Ok(())
}

fn test_all_features_for_package(
    package: &cargo_metadata::Package,
    command: crate::test_runner::CargoCommand,
    cargo_args: &[String],
) -> Result<TestOutcome, Box<dyn error::Error>> {
    let feature_sets = crate::features_finder::fetch_feature_sets(package);

    for feature_set in feature_sets {
        let mut test_runner = crate::test_runner::TestRunner::new(
            command,
            package.name.clone(),
            feature_set.clone(),
            cargo_args,
            package
                .manifest_path
                .parent()
                .expect("could not find parent of cargo manifest path")
                .to_owned(),
        );

        let outcome = test_runner.run()?;

        match outcome {
            TestOutcome::Pass => (),
            // Fail fast if we encounter a test failure
            t @ TestOutcome::Fail(_) => return Ok(t),
        }
    }

    Ok(TestOutcome::Pass)
}

fn determine_packages_to_test() -> Result<Vec<cargo_metadata::Package>, Box<dyn error::Error>> {
    let current_dir = env::current_dir()?;
    let metadata = cargo_metadata::fetch()?;

    Ok(if current_dir == metadata.workspace_root {
        metadata
            .packages
            .iter()
            .filter(|package| metadata.workspace_members.contains(&package.id))
            .cloned()
            .collect::<Vec<cargo_metadata::Package>>()
    } else {
        vec![metadata
            .packages
            .iter()
            .find(|package| package.manifest_path.parent() == Some(&current_dir))
            .expect("Could not find cargo package in metadata")
            .to_owned()]
    })
}

fn cargo_cmd() -> ffi::OsString {
    env::var_os("CARGO").unwrap_or_else(|| ffi::OsString::from("cargo"))
}

#[derive(Eq, PartialEq)]
pub enum TestOutcome {
    Pass,
    Fail(process::ExitStatus),
}
