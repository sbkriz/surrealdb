use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use anyhow::{Context as _, Result, bail};
use tokio::process::{Child, Command};

use crate::cli::Backend;

/// Manages external infrastructure required by each storage backend.
///
/// - **Memory**: no external infrastructure needed.
/// - **FileStore** (RocksDB, SurrealKV): creates and owns a temporary directory
///   for datastore files.
/// - **TikV**: starts a TiKV playground via `tiup` and manages its lifecycle.
pub enum BackendService {
	Memory,
	FileStore {
		dir: PathBuf,
	},
	TikV {
		child: Child,
		tiup: PathBuf,
		tiup_home: PathBuf,
	},
}

fn xorshift(state: &mut u32) -> u32 {
	let mut x = *state;
	x ^= x << 13;
	x ^= x >> 17;
	x ^= x << 5;
	*state = x;
	x
}

impl BackendService {
	pub async fn start(backend: Backend) -> Result<Self> {
		match backend {
			Backend::Memory => Ok(BackendService::Memory),
			Backend::RocksDb | Backend::SurrealKv => Self::start_file_store().await,
			Backend::TikV => Self::start_tikv().await,
		}
	}

	/// Base directory for file-based backends (RocksDB, SurrealKV).
	/// Returns `None` for Memory and TiKV.
	pub fn base_dir(&self) -> Option<&Path> {
		match self {
			BackendService::FileStore {
				dir,
			} => Some(dir),
			_ => None,
		}
	}

	pub async fn stop(self) -> Result<()> {
		match self {
			BackendService::Memory => Ok(()),
			BackendService::FileStore {
				dir,
			} => {
				if let Err(e) = tokio::fs::remove_dir_all(&dir).await {
					eprintln!("Failed to clean up temporary dir: {e}");
				}
				Ok(())
			}
			BackendService::TikV {
				mut child,
				tiup,
				tiup_home,
			} => Self::stop_tikv(&mut child, &tiup, &tiup_home).await,
		}
	}

	async fn start_file_store() -> Result<Self> {
		let temp_dir = std::env::temp_dir();
		let time = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap();
		let time = time.as_secs() ^ time.subsec_nanos() as u64;
		let mut state = (time >> 32) as u32 ^ time as u32;

		let rand = xorshift(&mut state);
		let mut dir = temp_dir.join(format!("surreal_lang_tests_{rand}"));

		while tokio::fs::metadata(&dir).await.is_ok() {
			let rand = xorshift(&mut state);
			dir = temp_dir.join(format!("surreal_lang_tests_{rand}"));
		}

		tokio::fs::create_dir(&dir).await?;
		eprintln!(" Using '{}' as temporary directory for datastores", dir.display());

		Ok(BackendService::FileStore {
			dir,
		})
	}

	fn find_runfiles_dir() -> Option<PathBuf> {
		if let Ok(dir) = std::env::var("RUNFILES_DIR") {
			let p = PathBuf::from(dir);
			if p.is_dir() {
				return Some(p);
			}
		}

		let exe = std::env::current_exe().ok()?;
		let name = exe.file_name()?.to_str()?;
		let runfiles = exe.with_file_name(format!("{name}.runfiles"));
		if runfiles.is_dir() {
			return Some(runfiles);
		}

		None
	}

	fn find_tiup() -> Result<PathBuf> {
		if let Some(runfiles) = Self::find_runfiles_dir() {
			// bzlmod canonical name (+tiup_repo+tiup) and apparent name (tiup)
			for repo_dir in ["+tiup_repo+tiup", "tiup"] {
				let tiup = runfiles.join(repo_dir).join("tiup");
				if tiup.exists() {
					return Ok(tiup);
				}
			}
		}

		if let Ok(output) = std::process::Command::new("which").arg("tiup").output() {
			if output.status.success() {
				let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
				if !path.is_empty() {
					return Ok(PathBuf::from(path));
				}
			}
		}

		if let Ok(home) = std::env::var("HOME") {
			let tiup = PathBuf::from(home).join(".tiup/bin/tiup");
			if tiup.exists() {
				return Ok(tiup);
			}
		}

		bail!(
			"tiup not found in runfiles, PATH, or at $HOME/.tiup/bin/tiup. \
			 Install it with: curl --proto '=https' --tlsv1.2 -sSf https://tiup-mirrors.pingcap.com/install.sh | sh"
		);
	}

	const TIUP_MIRROR: &str = "https://tiup-mirrors.pingcap.com";

	/// Build a tiup command with TIUP_HOME/TIUP_MIRRORS set.
	/// stdout is redirected to null because the worker uses stdout for the
	/// JSON protocol. stderr is inherited (goes to the Bazel worker log).
	fn tiup_cmd_async(tiup: &Path, tiup_home: &Path) -> Command {
		let mut cmd = Command::new(tiup);
		cmd.env("TIUP_HOME", tiup_home);
		cmd.env("TIUP_MIRRORS", Self::TIUP_MIRROR);
		cmd.stdout(std::process::Stdio::null());
		cmd.stderr(std::process::Stdio::inherit());
		cmd
	}

	async fn start_tikv() -> Result<Self> {
		let tiup = Self::find_tiup()?;

		let tiup_home = std::env::temp_dir().join(format!(
			"surreal_tiup_home_{}",
			std::process::id()
		));
		tokio::fs::create_dir_all(tiup_home.join("bin"))
			.await
			.context("creating TIUP_HOME/bin")?;
		eprintln!(" Using '{}' as TIUP_HOME", tiup_home.display());

		eprintln!("Initializing tiup mirror...");
		let status = Self::tiup_cmd_async(&tiup, &tiup_home)
			.args(["mirror", "set", Self::TIUP_MIRROR])
			.status()
			.await
			.context("failed to set tiup mirror")?;
		if !status.success() {
			bail!("tiup mirror set failed with {status}");
		}

		eprintln!("Installing TiKV playground components...");
		let status = Self::tiup_cmd_async(&tiup, &tiup_home)
			.args(["install", "pd", "tikv", "playground"])
			.status()
			.await
			.context("failed to run tiup install")?;
		if !status.success() {
			bail!("tiup install failed with {status}");
		}

		eprintln!("Cleaning stale TiKV playground...");
		let _ = Self::tiup_cmd_async(&tiup, &tiup_home).args(["clean", "--all"]).status().await;
		let _ = std::process::Command::new("killall").args(["-q", "tiup-playground"]).status();

		eprintln!("Starting TiKV playground...");
		let child = Self::tiup_cmd_async(&tiup, &tiup_home)
			.args([
				"playground",
				"--mode",
				"tikv-slim",
				"--kv",
				"1",
				"--pd",
				"1",
				"--db",
				"0",
				"--ticdc",
				"0",
				"--tiflash",
				"0",
				"--without-monitor",
				"--tag",
				"testing",
			])
			.spawn()
			.context("failed to spawn tiup playground")?;

		eprintln!("Waiting for TiKV playground to start...");
		let mut ready = false;
		for attempt in 1..=5 {
			tokio::time::sleep(Duration::from_secs(5)).await;
			eprintln!("  Checking playground status (attempt {attempt}/5)...");
			let result = Self::tiup_cmd_async(&tiup, &tiup_home)
				.args(["playground", "display"])
				.status()
				.await;
			if let Ok(s) = result {
				if s.success() {
					ready = true;
					break;
				}
			}
		}

		if !ready {
			bail!("TiKV playground failed to start after 5 attempts (25s)");
		}

		eprintln!("TiKV playground is ready");
		Ok(BackendService::TikV {
			child,
			tiup,
			tiup_home,
		})
	}

	async fn stop_tikv(child: &mut Child, tiup: &Path, tiup_home: &Path) -> Result<()> {
		eprintln!("Stopping TiKV playground...");

		let _ = child.kill().await;
		let _ = child.wait().await;

		let _ = Self::tiup_cmd_async(tiup, tiup_home).args(["clean", "--all"]).status().await;
		let _ = std::process::Command::new("killall").args(["-q", "tiup-playground"]).status();

		if let Err(e) = tokio::fs::remove_dir_all(tiup_home).await {
			eprintln!("Failed to clean up TIUP_HOME: {e}");
		}

		Ok(())
	}
}
