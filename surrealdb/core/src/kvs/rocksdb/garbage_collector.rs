use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::Duration;

use parking_lot::Mutex;
use rocksdb::OptimisticTransactionDB;
use web_time::{SystemTime, UNIX_EPOCH};

use super::TARGET;
use crate::kvs::err::{Error, Result};

const GC_INTERVAL: Duration = Duration::from_secs(60);

/// Background garbage collector for removing old versioned key entries.
///
/// When user-defined timestamps (versioning) are enabled with a finite retention period,
/// RocksDB retains all historical versions of every key. This component runs a dedicated
/// background thread that periodically advances the `full_history_ts_low` watermark,
/// allowing automatic compaction to drop versions older than the configured retention period.
///
/// ## Configuration
///
/// Garbage collection is activated when both conditions are met:
/// - Versioning is enabled via `versioned=true`
/// - A finite retention period is set via `retention=<duration>` (e.g. `retention=7d`)
///
/// ## Behavior
///
/// Every 60 seconds the background thread:
/// 1. Computes a GC threshold timestamp based on `now - retention_period`
/// 2. Advances the RocksDB `full_history_ts_low` watermark on the default column family
/// 3. Automatic compaction then drops versions with timestamps below the watermark
///
/// The watermark is monotonically increasing -- RocksDB silently rejects attempts to
/// lower it, so clock adjustments are handled safely.
pub struct GarbageCollector {
	/// Shutdown flag
	shutdown: Arc<AtomicBool>,
	/// Thread handle
	handle: Mutex<Option<thread::JoinHandle<()>>>,
}

impl GarbageCollector {
	/// Create a new garbage collector that advances the GC watermark every 60 seconds.
	pub fn new(db: Pin<Arc<OptimisticTransactionDB>>, retention_ns: u64) -> Result<Self> {
		let retention_millis = retention_ns / 1_000_000;
		info!(target: TARGET, "Version garbage collector: enabled (retention={}ms, interval={}s)",
			retention_millis,
			GC_INTERVAL.as_secs(),
		);
		// Create a new shutdown flag
		let shutdown = Arc::new(AtomicBool::new(false));
		// Clone the shutdown flag
		let finished = shutdown.clone();
		// Spawn the garbage collector thread
		let handle = thread::Builder::new()
			.name("rocksdb-garbage-collector".to_string())
			.spawn(move || {
				loop {
					// Wait for the GC interval
					thread::sleep(GC_INTERVAL);
					// Check shutdown flag after sleep
					if finished.load(Ordering::Relaxed) {
						break;
					}
					// Compute the GC threshold as an HLC timestamp
					let now_millis = SystemTime::now()
						.duration_since(UNIX_EPOCH)
						.expect("system time cannot be before epoch")
						.as_millis() as u64;
					let threshold_millis = now_millis.saturating_sub(retention_millis);
					let threshold_ts = threshold_millis << 16;
					let ts_bytes = threshold_ts.to_le_bytes();
					// Get the default column family handle
					let Some(cf) = db.cf_handle("default") else {
						error!(target: TARGET, "Failed to get default column family handle for GC");
						continue;
					};
					// Advance the full_history_ts_low watermark
					if let Err(err) = db.increase_full_history_ts_low(cf, ts_bytes) {
						error!(target: TARGET, "Failed to advance GC watermark: {err}");
					} else {
						trace!(target: TARGET, "Advanced GC watermark to {threshold_ts} (threshold={}ms ago)", retention_millis);
					}
				}
			})
			.map_err(|_| {
				Error::Datastore(
					"failed to spawn RocksDB garbage collector thread".to_string(),
				)
			})?;
		// Create a new garbage collector
		Ok(Self {
			shutdown,
			handle: Mutex::new(Some(handle)),
		})
	}

	/// Shutdown the garbage collector
	pub fn shutdown(&self) -> Result<()> {
		// Signal shutdown
		self.shutdown.store(true, Ordering::Relaxed);
		// Wait for thread to finish
		if let Some(handle) = self.handle.lock().take() {
			let _ = handle.join();
		}
		// All good
		Ok(())
	}
}
