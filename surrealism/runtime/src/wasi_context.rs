use std::path::Path;

use anyhow::Result;
use surrealism_types::err::PrefixErr;
use wasmtime::component::ResourceTable;
use wasmtime_wasi::p1::WasiP1Ctx;
use wasmtime_wasi::{DirPerms, FilePerms, WasiCtx, WasiCtxBuilder};

pub fn build_p1(fs_root: Option<&Path>) -> Result<WasiP1Ctx> {
	let mut builder = WasiCtxBuilder::new();
	builder.inherit_stdout().inherit_stderr();
	if let Some(root) = fs_root {
		builder
			.preopened_dir(root, "/", DirPerms::READ, FilePerms::READ)
			.prefix_err(|| "Failed to preopen filesystem directory")?;
	}
	Ok(builder.build_p1())
}

pub fn build_p2(fs_root: Option<&Path>) -> Result<(WasiCtx, ResourceTable)> {
	let mut builder = WasiCtxBuilder::new();
	builder.inherit_stdout().inherit_stderr();
	if let Some(root) = fs_root {
		builder
			.preopened_dir(root, "/", DirPerms::READ, FilePerms::READ)
			.prefix_err(|| "Failed to preopen filesystem directory")?;
	}
	Ok((builder.build(), ResourceTable::new()))
}
