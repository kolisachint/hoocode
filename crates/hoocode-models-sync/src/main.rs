//! `models-sync`: regenerates the hoocode model catalog from models.dev.
//!
//! Run from the workspace (see `crates/hoocode-ai-models-catalog/README.md`):
//!
//! ```text
//! cargo run -p hoocode-models-sync --bin models-sync
//! cargo run -p hoocode-models-sync --bin models-sync -- --input api.json --dry-run
//! ```

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use hoocode_models_sync::{parse_catalog, sync, to_json, Overrides, Upstream, MODELS_DEV_URL};

const USAGE: &str = "\
usage: models-sync [--input FILE] [--data-dir DIR] [--dry-run]

  --input FILE    read models.dev's api.json from FILE instead of fetching it
  --data-dir DIR  catalog data directory (default: the catalog crate's data/)
  --dry-run       print the report and write nothing
";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("models-sync: {message}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut input = None;
    let mut data_dir = default_data_dir();
    let mut dry_run = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--input" => input = Some(PathBuf::from(value(&mut args, "--input")?)),
            "--data-dir" => data_dir = PathBuf::from(value(&mut args, "--data-dir")?),
            "--dry-run" => dry_run = true,
            "-h" | "--help" => {
                print!("{USAGE}");
                return Ok(());
            }
            other => return Err(format!("unknown argument `{other}`\n{USAGE}")),
        }
    }

    let models_path = data_dir.join("models.json");
    let baseline_text = read(&models_path)?;
    let baseline =
        parse_catalog(&baseline_text).map_err(|e| format!("{}: {e}", models_path.display()))?;
    let overrides_path = data_dir.join("overrides.json");
    let overrides: Overrides = serde_json::from_str(&read(&overrides_path)?)
        .map_err(|e| format!("{}: {e}", overrides_path.display()))?;

    let upstream: Upstream = match &input {
        Some(path) => {
            let text = read(path)?;
            serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?
        }
        None => fetch(MODELS_DEV_URL)?,
    };

    let result = sync(&baseline, &upstream, &overrides)?;
    print!("{}", result.report.render());

    let fresh = to_json(&result.models);
    if dry_run {
        println!("\ndry run: {} not written", models_path.display());
    } else if fresh == baseline_text {
        println!("\nno change: {} is up to date", models_path.display());
    } else {
        std::fs::write(&models_path, fresh)
            .map_err(|e| format!("write {}: {e}", models_path.display()))?;
        println!("\nwrote {}", models_path.display());
    }
    Ok(())
}

fn value(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
    args.next().ok_or_else(|| format!("{flag} needs a value"))
}

fn read(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))
}

/// `crates/hoocode-ai-models-catalog/data`, found from this crate's manifest.
fn default_data_dir() -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../hoocode-ai-models-catalog/data");
    dir.canonicalize().unwrap_or(dir)
}

fn fetch(url: &str) -> Result<Upstream, String> {
    let client = reqwest::blocking::Client::builder()
        .user_agent(concat!("hoocode-models-sync/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| format!("http client: {e}"))?;
    let body = client
        .get(url)
        .send()
        .and_then(|r| r.error_for_status())
        .and_then(|r| r.text())
        .map_err(|e| format!("fetch {url}: {e}"))?;
    serde_json::from_str(&body).map_err(|e| format!("parse {url}: {e}"))
}
