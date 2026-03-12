use semver::Version;
use serde::{Deserialize, Serialize};
use surrealism_types::err::{PrefixErr, SurrealismResult};

use crate::capabilities::SurrealismCapabilities;

/// Which WASM ABI the plugin targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AbiVersion {
	/// WASI Preview 1 (core module with linear memory ABI).
	P1,
	/// WASI Preview 2 (component model).
	#[default]
	P2,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SurrealismConfig {
	#[serde(rename = "package")]
	pub meta: SurrealismMeta,
	#[serde(default)]
	pub capabilities: SurrealismCapabilities,
	#[serde(default)]
	pub abi: AbiVersion,
	#[serde(default)]
	pub attach: SurrealismAttach,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct SurrealismAttach {
	/// Path to a directory whose contents are bundled into the archive and
	/// mounted as a read-only filesystem for the WASM module. Can be relative
	/// (resolved against the project root at build time) or absolute.
	pub fs: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SurrealismMeta {
	pub organisation: String,
	pub name: String,
	pub version: Version,
}

impl SurrealismConfig {
	pub fn parse(s: &str) -> SurrealismResult<Self> {
		toml::from_str(s).prefix_err(|| "Failed to parse Surrealism config")
	}

	pub fn to_string(&self) -> SurrealismResult<String> {
		toml::to_string(self).prefix_err(|| "Failed to serialize Surrealism config")
	}

	pub fn file_name(&self) -> String {
		format!("{}-{}-{}.surli", self.meta.organisation, self.meta.name, self.meta.version)
	}
}
