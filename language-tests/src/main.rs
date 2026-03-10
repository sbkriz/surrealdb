#![recursion_limit = "256"]

mod cli;
mod cmd;
mod format;
mod runner;
mod temp_dir;
mod tests;

use anyhow::{self, Result};
use cli::ColorMode;

/// Expand @flagfile references in argument lists (Bazel worker convention).
fn expand_flagfiles(args: &[String]) -> Vec<String> {
	let mut result = Vec::new();
	for arg in args {
		if let Some(path) = arg.strip_prefix('@') {
			match std::fs::read_to_string(path) {
				Ok(contents) => {
					for line in contents.lines() {
						let line = line.trim();
						if !line.is_empty() {
							result.push(line.to_string());
						}
					}
				}
				Err(e) => {
					eprintln!("Warning: failed to read flagfile '{path}': {e}");
					result.push(arg.clone());
				}
			}
		} else {
			result.push(arg.clone());
		}
	}
	result
}

#[tokio::main]
async fn main() -> Result<()> {
	// When both rust_crypto and aws_lc_rs features are active (Bazel builds),
	// jsonwebtoken cannot auto-detect the provider. Install aws_lc explicitly.
	// On Cargo builds (single feature active), this is harmless -- install_default
	// returns Err if auto-detection already set a provider.
	cfg_if::cfg_if! {
		if #[cfg(not(target_family = "wasm"))] {
			let _ = jsonwebtoken::crypto::aws_lc::DEFAULT_PROVIDER.install_default();
		}
	}

	let raw_args: Vec<String> = std::env::args().collect();

	// Bazel persistent worker mode: the binary is invoked with --persistent_worker.
	if raw_args.iter().any(|a| a == "--persistent_worker") {
		return cmd::run::worker::run_persistent().await;
	}

	// Bazel non-worker fallback: the binary is invoked with @flagfile args.
	// Expand flagfiles and check for worker-style arguments.
	if raw_args[1..].iter().any(|a| a.starts_with('@') || a == "--result-file") {
		let expanded = expand_flagfiles(&raw_args[1..]);
		return cmd::run::worker::run_single(&expanded).await;
	}

	// Normal CLI mode (cargo-based test runner).
	let matches = cli::parse();

	let color: ColorMode = matches.get_one("color").copied().unwrap();

	let (sub, args) = matches.subcommand().unwrap();

	match sub {
		"test" => cmd::run::run(color, args).await,
		#[cfg(not(feature = "upgrade"))]
		"upgrade" => {
			anyhow::bail!(
				"Upgrade subcommand is only implemented when the 'upgrade' feature is enabled"
			)
		}
		#[cfg(feature = "upgrade")]
		"upgrade" => cmd::upgrade::run(color, args).await,
		"list" => cmd::list::run(args).await,
		_ => unreachable!(),
	}
}
