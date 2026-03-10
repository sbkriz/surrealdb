use std::collections::HashMap;
use std::io::Write as _;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context as _, Result};
use semver::Version;
use serde::{Deserialize, Serialize};
use surrealdb_core::dbs::Session;
use surrealdb_core::env::VERSION;
use surrealdb_core::kvs::Datastore;
use tokio::io::AsyncBufReadExt as _;
use tokio::sync::mpsc;

use crate::cli::Backend;
use crate::tests::TestSet;
use crate::tests::report::{TestGrade, TestReport, TestTaskResult};
use crate::tests::schema::TestConfig;
use crate::tests::set::TestId;

use super::backend_service::BackendService;
use super::provisioner::{PermitError, Permit, Provisioner};
use super::util::core_capabilities_from_test_config;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkRequest {
	arguments: Vec<String>,
	#[serde(default)]
	request_id: i32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkResponse {
	exit_code: i32,
	output: String,
	request_id: i32,
}

struct RequestArgs {
	backend: Backend,
	test_root: String,
	test_file: String,
	result_file: String,
}

impl RequestArgs {
	fn parse(args: &[String]) -> Result<Self> {
		let mut backend = None;
		let mut test_root = None;
		let mut test_file = None;
		let mut result_file = None;

		let mut iter = args.iter();
		while let Some(arg) = iter.next() {
			match arg.as_str() {
				"--backend" => backend = iter.next().cloned(),
				"--test-root" => test_root = iter.next().cloned(),
				"--test-file" => test_file = iter.next().cloned(),
				"--result-file" => result_file = iter.next().cloned(),
				_ => {}
			}
		}

		Ok(RequestArgs {
			backend: Backend::from_arg(
				backend.as_deref().context("missing --backend argument")?,
			)
			.map_err(|e| anyhow::anyhow!(e))?,
			test_root: test_root.context("missing --test-root argument")?,
			test_file: test_file.context("missing --test-file argument")?,
			result_file: result_file.context("missing --result-file argument")?,
		})
	}
}

/// Bazel persistent worker mode (supports both singleplex and multiplex).
///
/// Architecture for concurrent request handling:
/// - **Reader task**: reads WorkRequests from stdin into a channel
/// - **Dispatcher loop**: owns the Provisioner, caches TestSets, obtains
///   datastore permits, and spawns per-request execution tasks
/// - **Execution tasks**: run individual tests concurrently against pooled
///   datastores and send WorkResponses to the writer channel
/// - **Writer task**: serialises WorkResponses to stdout
pub(crate) async fn run_persistent() -> Result<()> {
	let matching_ds = Arc::new(create_matching_datastore().await?);

	let (resp_tx, resp_rx) = mpsc::unbounded_channel::<WorkResponse>();
	let writer_handle = tokio::spawn(write_responses(resp_rx));

	let (req_tx, mut req_rx) = mpsc::channel::<WorkRequest>(32);
	tokio::spawn(read_requests(req_tx));

	let mut service: Option<BackendService> = None;
	let mut provisioner: Option<Provisioner> = None;
	let mut testset_cache: HashMap<String, TestSet> = HashMap::new();

	while let Some(request) = req_rx.recv().await {
		let request_id = request.request_id;

		let req_args = match RequestArgs::parse(&request.arguments) {
			Ok(a) => a,
			Err(e) => {
				let _ = resp_tx.send(WorkResponse {
					exit_code: 1,
					output: format!("{e:?}"),
					request_id,
				});
				continue;
			}
		};

		if provisioner.is_none() {
			let svc = match BackendService::start(req_args.backend).await {
				Ok(s) => s,
				Err(e) => {
					let _ = resp_tx.send(WorkResponse {
						exit_code: 1,
						output: format!("failed to start backend service: {e:?}"),
						request_id,
					});
					continue;
				}
			};

			let pool_size =
				std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
			match Provisioner::new(pool_size, req_args.backend, svc.base_dir()).await {
				Ok(p) => {
					service = Some(svc);
					provisioner = Some(p);
				}
				Err(e) => {
					if let Err(stop_err) = svc.stop().await {
						eprintln!("failed to stop backend service after provisioner error: {stop_err:?}");
					}
					let _ = resp_tx.send(WorkResponse {
						exit_code: 1,
						output: format!("failed to create provisioner: {e:?}"),
						request_id,
					});
					continue;
				}
			}
		}

		if !testset_cache.contains_key(&req_args.test_root) {
			match TestSet::collect_directory(&req_args.test_root).await {
				Ok((ts, _)) => {
					testset_cache.insert(req_args.test_root.clone(), ts);
				}
				Err(e) => {
					let _ = resp_tx.send(WorkResponse {
						exit_code: 1,
						output: format!("failed to load tests from '{}': {e:?}", req_args.test_root),
						request_id,
					});
					continue;
				}
			}
		}
		let testset = testset_cache.get(&req_args.test_root).unwrap().clone();

		let id = match testset.find_all(&req_args.test_file) {
			Some(id) => id,
			None => {
				let msg = format!("test '{}' not found in '{}'", req_args.test_file, req_args.test_root);
				let _ = write_result(&req_args.result_file, false, &msg);
				let _ = resp_tx.send(WorkResponse {
					exit_code: 0,
					output: String::new(),
					request_id,
				});
				continue;
			}
		};

		let test = &testset[id];
		let config = test.config.as_ref();

		if test.contains_error
			|| !config.should_run()
			|| is_backend_filtered(config, req_args.backend)
			|| should_skip_test(&testset, id, config)
		{
			let _ = write_result(&req_args.result_file, true, "");
			let _ = resp_tx.send(WorkResponse {
				exit_code: 0,
				output: String::new(),
				request_id,
			});
			continue;
		}

		let prov = provisioner.as_mut().unwrap();
		let versioned = config.env.as_ref().map(|e| e.versioned).unwrap_or(false);
		let permit = if config.can_use_reusable_ds() {
			prov.obtain().await
		} else {
			prov.create(versioned)
		};

		let tx = resp_tx.clone();
		let ds = matching_ds.clone();
		tokio::spawn(async move {
			let (passed, output) = run_test_task(&req_args, permit, &testset, id, &ds).await;
			let _ = write_result(&req_args.result_file, passed, &output);
			let _ = tx.send(WorkResponse {
				exit_code: 0,
				output: String::new(),
				request_id,
			});
		});
	}

	drop(resp_tx);
	let _ = writer_handle.await;

	if let Some(p) = provisioner {
		p.shutdown().await?;
	}
	if let Some(svc) = service {
		svc.stop().await?;
	}

	Ok(())
}

/// Single-test mode: Bazel non-worker fallback. The binary is invoked
/// directly with the same arguments that would be in a WorkRequest.
pub(crate) async fn run_single(args: &[String]) -> Result<()> {
	let req_args = RequestArgs::parse(args)?;
	let matching_ds = create_matching_datastore().await?;
	let service = BackendService::start(req_args.backend).await?;
	let mut provisioner = Provisioner::new(1, req_args.backend, service.base_dir()).await?;

	let (testset, _) = TestSet::collect_directory(&req_args.test_root).await?;
	let id = testset
		.find_all(&req_args.test_file)
		.context(format!("test '{}' not found in '{}'", req_args.test_file, req_args.test_root))?;

	let config = testset[id].config.as_ref();
	let (passed, output) = if testset[id].contains_error
		|| !config.should_run()
		|| is_backend_filtered(config, req_args.backend)
		|| should_skip_test(&testset, id, config)
	{
		(true, String::new())
	} else {
		let versioned = config.env.as_ref().map(|e| e.versioned).unwrap_or(false);
		let permit = if config.can_use_reusable_ds() {
			provisioner.obtain().await
		} else {
			provisioner.create(versioned)
		};
		run_test_task(&req_args, permit, &testset, id, &matching_ds).await
	};

	write_result(&req_args.result_file, passed, &output)?;
	provisioner.shutdown().await?;
	service.stop().await?;
	Ok(())
}

fn is_backend_filtered(config: &crate::tests::schema::TestConfig, backend: Backend) -> bool {
	if let Some(env) = &config.env {
		let backend_str = backend.to_string();
		if !env.backend.is_empty() && !env.backend.contains(&backend_str) {
			return true;
		}
	}
	false
}

/// Check whether a test should be skipped due to version constraints or upgrade requirements.
/// Mirrors the filtering logic in the non-worker `run()` path (mod.rs).
fn should_skip_test(testset: &TestSet, id: TestId, config: &TestConfig) -> bool {
	let core_version = Version::parse(VERSION).unwrap();

	if let Some(version_req) = config.test.as_ref().and_then(|x| x.version.as_ref()) {
		if !version_req.matches(&core_version) {
			return true;
		}
	}

	if let Some(version_req) = config.test.as_ref().and_then(|x| x.importing_version.as_ref()) {
		if !version_req.matches(&core_version) {
			return true;
		}
	}

	for import in testset[id].imports.iter() {
		if let Some(version_req) =
			testset[import.id].config.test.as_ref().and_then(|x| x.version.as_ref())
		{
			if !version_req.matches(&core_version) {
				return true;
			}
		}
	}

	if config.test.as_ref().map(|x| x.upgrade).unwrap_or(false) {
		return true;
	}

	false
}

async fn read_requests(tx: mpsc::Sender<WorkRequest>) {
	let stdin = tokio::io::stdin();
	let mut reader = tokio::io::BufReader::new(stdin);
	let mut line = String::new();

	loop {
		line.clear();
		match reader.read_line(&mut line).await {
			Ok(0) => break,
			Ok(_) => {}
			Err(e) => {
				eprintln!("worker: stdin read error: {e}");
				break;
			}
		}
		let trimmed = line.trim();
		if trimmed.is_empty() {
			continue;
		}

		match serde_json::from_str::<WorkRequest>(trimmed) {
			Ok(req) => {
				if tx.send(req).await.is_err() {
					break;
				}
			}
			Err(e) => {
				eprintln!("worker: failed to parse WorkRequest: {e}");
			}
		}
	}
}

async fn write_responses(mut rx: mpsc::UnboundedReceiver<WorkResponse>) {
	while let Some(resp) = rx.recv().await {
		let mut stdout = std::io::stdout().lock();
		if let Err(e) = serde_json::to_writer(&mut stdout, &resp) {
			eprintln!("worker: failed to write response: {e}");
			continue;
		}
		let _ = writeln!(stdout);
		let _ = stdout.flush();
	}
}

/// Execute a single test using a pre-obtained datastore permit.
async fn run_test_task(
	args: &RequestArgs,
	permit: Permit,
	testset: &TestSet,
	id: crate::tests::set::TestId,
	matching_ds: &Datastore,
) -> (bool, String) {
	match run_test_task_inner(args, permit, testset, id, matching_ds).await {
		Ok(v) => v,
		Err(e) => (false, format!("Internal error: {e:?}")),
	}
}

async fn run_test_task_inner(
	args: &RequestArgs,
	permit: Permit,
	testset: &TestSet,
	id: crate::tests::set::TestId,
	matching_ds: &Datastore,
) -> Result<(bool, String)> {
	let config = testset[id].config.as_ref();
	let capabilities = core_capabilities_from_test_config(config);
	let backend_str = args.backend.to_string();

	let context_timeout_duration = config
		.env
		.as_ref()
		.map(|x| {
			x.context_timeout(Some(&backend_str))
				.map(Duration::from_millis)
				.unwrap_or(Duration::MAX)
		})
		.unwrap_or(Duration::from_secs(3));

	let backend = args.backend;
	let test_result = permit
		.with(
			move |ds| {
				ds.with_capabilities(capabilities)
					.with_query_timeout(Some(context_timeout_duration))
			},
			async |ds| super::run_test_with_dbs(id, testset, ds, backend).await,
		)
		.await;

	let task_result = match test_result {
		Ok(Ok(r)) => r,
		Ok(Err(e)) => return Err(e),
		Err(PermitError::Other(e)) => return Err(e),
		Err(PermitError::Panic(e)) => TestTaskResult::Panicked(e),
	};

	let report =
		TestReport::from_test_result(id, testset, task_result, matching_ds, None).await;

	let grade = report.grade();
	let output = report.display_to_string(testset);
	let passed = !matches!(grade, TestGrade::Failed);

	Ok((passed, output))
}

fn write_result(path: &str, passed: bool, output: &str) -> Result<()> {
	let status = if passed { "PASS" } else { "FAIL" };
	let contents = if output.is_empty() {
		format!("{status}\n")
	} else {
		format!("{status}\n{output}")
	};
	std::fs::write(path, contents).context("writing result file")?;
	Ok(())
}

async fn create_matching_datastore() -> Result<Datastore> {
	let ds = Datastore::new("memory")
		.await
		.context("creating matching datastore")?;
	let mut session = Session::default();
	ds.process_use(None, &mut session, Some("match".to_string()), Some("match".to_string()))
		.await
		.context("setting up matching datastore session")?;
	Ok(ds)
}
