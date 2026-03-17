mod console;
mod logs;
pub mod metrics;
pub mod traces;

use std::collections::HashMap;
use std::net::ToSocketAddrs;
use std::sync::{Arc, LazyLock};

use anyhow::{Result, anyhow};
use opentelemetry::global;
use opentelemetry_sdk::Resource;
use surrealdb_core::CommunityComposer;
use tracing::{Level, Subscriber};
use tracing_appender::non_blocking::{NonBlockingBuilder, WorkerGuard};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::filter::{LevelFilter, ParseError};
use tracing_subscriber::layer::Filter;
use tracing_subscriber::prelude::*;
use tracing_subscriber::registry::LookupSpan;

use crate::cli::LogFormat;
pub use crate::cli::validator::parser::tracing::{CustomFilter, SpanFilter};
use crate::cnf::ENABLE_TOKIO_CONSOLE;

/// Identifies the logging destination that a filter is being configured for.
///
/// This is passed to [`LoggingComposer::with_filter`] so that the composer can
/// apply destination-specific filter customisations.
pub enum LoggingType {
	/// Standard I/O output (stdout for normal messages, stderr for warnings/errors).
	Stdio,
	/// Rotating log-file output.
	File,
	/// Remote socket (TCP) output.
	Socket,
	/// OpenTelemetry tracing exporter.
	Otel,
}

/// Trait that allows an external composer to customise the logging filters
/// applied to each telemetry destination.
///
/// Every logging destination (stdio, file, socket, OpenTelemetry) calls
/// [`with_filter`](Self::with_filter) before building its layer, giving the
/// implementor a chance to replace or transform both the [`EnvFilter`] and the
/// span-level [`Filter`].
///
/// The default implementation — provided for [`CommunityComposer`] — returns
/// the filters unchanged.
pub trait LoggingComposer {
	/// Optionally transform the [`CustomFilter`] for the given logging destination.
	///
	/// The caller supplies a [`LoggingType`] that identifies which destination
	/// the filter will be applied to, together with the [`CustomFilter`] parsed
	/// from the CLI / environment. The implementor returns:
	///
	/// * An [`EnvFilter`] — module-level directive filter.
	/// * A [`Filter<S>`] — span-level filter.
	///
	/// The default implementation returns the filters from `custom_filter`
	/// unchanged.
	fn with_filter<S>(
		&mut self,
		_logging_type: LoggingType,
		custom_filter: CustomFilter,
	) -> (EnvFilter, impl Filter<S> + Send + Sync + 'static)
	where
		S: Subscriber + for<'a> LookupSpan<'a> + Send + Sync,
	{
		(custom_filter.env(), custom_filter.span_filter())
	}
}

impl LoggingComposer for CommunityComposer {}

pub static OTEL_DEFAULT_RESOURCE: LazyLock<Resource> = LazyLock::new(|| {
	// Build resource from environment variables and default attributes
	// The Resource will automatically merge SDK, environment, and telemetry metadata
	Resource::builder().with_service_name("surrealdb").build()
});

/// Builder for constructing the telemetry subscriber stack.
///
/// Collects configuration for every logging destination (stdio, file, socket,
/// OpenTelemetry) and assembles them into a layered [`tracing::Subscriber`]
/// via [`build`](Self::build) or [`init`](Self::init).
#[derive(Debug, Clone)]
pub struct Builder {
	/// Output format for the stdio (terminal) layer.
	format: LogFormat,
	/// Global / default log filter applied when no destination-specific filter
	/// is set.
	filter: CustomFilter,
	/// Optional remote socket address for TCP log streaming.
	socket: Option<String>,
	// — Destination-specific filter overrides ————————————————
	/// Optional filter override for the file destination.
	file_filter: Option<CustomFilter>,
	/// Optional filter override for the OpenTelemetry destination.
	otel_filter: Option<CustomFilter>,
	/// Optional filter override for the socket destination.
	socket_filter: Option<CustomFilter>,
	// — Socket options ————————————————————————————————————————
	/// Output format for the socket layer.
	socket_format: LogFormat,
	// — File options ——————————————————————————————————————————
	/// Whether file logging is enabled.
	file_enabled: bool,
	/// Output format for the file layer.
	file_format: LogFormat,
	/// Directory path for log files.
	file_path: Option<String>,
	/// File name for the log file.
	file_name: Option<String>,
	/// Rotation interval (`"daily"`, `"hourly"`, or `"never"`).
	file_rotation: Option<String>,
}

/// Create a new [`Builder`] with default settings.
pub fn builder() -> Builder {
	Builder::default()
}

impl Default for Builder {
	fn default() -> Self {
		Self {
			filter: CustomFilter::default(),
			format: LogFormat::Text,
			socket: None,
			// Filter options
			file_filter: None,
			otel_filter: None,
			socket_filter: None,
			// Socket options
			socket_format: LogFormat::Text,
			// File options
			file_format: LogFormat::Text,
			file_enabled: false,
			file_path: Some("logs".to_string()),
			file_name: Some("surrealdb.log".to_string()),
			file_rotation: Some("daily".to_string()),
		}
	}
}

impl Builder {
	/// Install the tracing dispatcher globally.
	///
	/// Builds the full telemetry subscriber stack via [`Self::build`] and sets it
	/// as the global default tracing subscriber. The `composer` is used to obtain
	/// the base [`Registry`] through the [`LoggingComposer`] trait.
	pub fn init<C: LoggingComposer>(self, composer: &mut C) -> Result<Vec<WorkerGuard>> {
		// Setup logs, tracing, and metrics
		let (registry, guards) = self.build(composer)?;
		// Initialise the registry
		registry.init();
		// Everything ok
		Ok(guards)
	}

	/// Set the log filter on the builder
	pub fn with_filter(mut self, filter: CustomFilter) -> Self {
		self.filter = filter;
		self
	}

	/// Set the log level on the builder
	pub fn with_log_level(mut self, log_level: &str) -> Self {
		if let Ok(env_filter) = env_filter_from_value(log_level) {
			self.filter = CustomFilter::new(env_filter, Arc::new(HashMap::new()));
		}
		self
	}

	/// Set a custom log filter for file output
	pub fn with_file_filter(mut self, filter: Option<CustomFilter>) -> Self {
		self.file_filter = filter;
		self
	}

	/// Set a custom log filter for otel output
	pub fn with_otel_filter(mut self, filter: Option<CustomFilter>) -> Self {
		self.otel_filter = filter;
		self
	}

	/// Set a custom log filter for socket output
	pub fn with_socket_filter(mut self, filter: Option<CustomFilter>) -> Self {
		self.socket_filter = filter;
		self
	}

	/// Send logs to the provided socket address
	pub fn with_socket(mut self, socket: Option<String>) -> Self {
		self.socket = socket;
		self
	}

	/// Set the terminal log output format
	pub fn with_log_format(mut self, format: LogFormat) -> Self {
		self.format = format;
		self
	}

	/// Set the log file output format
	pub fn with_file_format(mut self, format: LogFormat) -> Self {
		self.file_format = format;
		self
	}

	/// Set the terminal log output format
	pub fn with_socket_format(mut self, format: LogFormat) -> Self {
		self.format = format;
		self
	}

	/// Enable or disable the log file
	pub fn with_file_enabled(mut self, enabled: bool) -> Self {
		self.file_enabled = enabled;
		self
	}

	/// Set the log file path
	pub fn with_file_path(mut self, path: Option<String>) -> Self {
		self.file_path = path;
		self
	}

	/// Set the log file name
	pub fn with_file_name(mut self, name: Option<String>) -> Self {
		self.file_name = name;
		self
	}

	/// Set the log file rotation interval (daily, hourly, or never)
	pub fn with_file_rotation(mut self, rotation: Option<String>) -> Self {
		self.file_rotation = rotation;
		self
	}

	/// Build a tracing dispatcher with the configured logging layers.
	///
	/// Constructs a [`tracing_subscriber::Registry`] and layers the configured
	/// stdio, file, socket, OpenTelemetry, and Tokio-console subscribers on top
	/// of it. Each layer's filter is passed through
	/// [`LoggingComposer::with_filter`] so that the `composer` can customise
	/// filtering per destination.
	pub fn build<C: LoggingComposer>(
		&self,
		composer: &mut C,
	) -> Result<(Box<dyn Subscriber + Send + Sync + 'static>, Vec<WorkerGuard>)> {
		// Setup the metrics layer
		if let Some(provider) = metrics::init()? {
			global::set_meter_provider(provider);
		}
		// Create a non-blocking stdout log destination
		let (stdout, stdout_guard) = NonBlockingBuilder::default()
			.lossy(true)
			.thread_name("surrealdb-logger-stdout")
			.finish(std::io::stdout());
		// Create a non-blocking stderr log destination
		let (stderr, stderr_guard) = NonBlockingBuilder::default()
			.lossy(true)
			.thread_name("surrealdb-logger-stderr")
			.finish(std::io::stderr());
		// Create the display destination layer
		let (env_filter, span_filter) =
			composer.with_filter(LoggingType::Stdio, self.filter.clone());
		let stdio_layer = logs::output(env_filter, span_filter, stdout, stderr, self.format)?;
		// Setup a registry for composing layers
		let registry = tracing_subscriber::registry();
		// Setup stdio destination layer
		let registry = registry.with(stdio_layer);
		// Setup guards
		let mut guards = vec![stdout_guard, stderr_guard];
		// Setup layers
		let mut layers = Vec::new();

		// Setup logging to opentelemetry
		{
			// Get the otel filter or global filter
			let (env_filter, span_filter) = composer.with_filter(
				LoggingType::Otel,
				self.otel_filter.clone().unwrap_or_else(|| self.filter.clone()),
			);
			// Create the otel destination layer
			if let Some(layer) = traces::new(env_filter, span_filter)? {
				// Add the layer to the registry
				layers.push(layer);
			}
		}

		// Setup logging to socket if enabled
		if let Some(addr) = &self.socket {
			// Parse the first socket address
			let address =
				addr.to_socket_addrs()?.next().ok_or_else(|| anyhow!("No matching addresses"))?;
			// Connect to the socket address
			let socket = logs::socket::connect(address)?;
			// Create a non-blocking socket log destination
			let (writer, guard) = NonBlockingBuilder::default()
				.lossy(false)
				.thread_name("surrealdb-logger-socket")
				.finish(socket);
			// Get the socket filter or global filter
			let (env_filter, span_filter) = composer.with_filter(
				LoggingType::Socket,
				self.socket_filter.clone().unwrap_or_else(|| self.filter.clone()),
			);
			// Create the socket destination layer
			let layer = logs::file(env_filter, span_filter, writer, self.socket_format)?;
			// Add the layer to the registry
			layers.push(layer);
			// Add the guard to the guards
			guards.push(guard);
		}

		// Setup logging to file if enabled
		if self.file_enabled {
			// Create the file appender based on rotation setting
			let file_appender = {
				// Parse the path and name
				let path = self.file_path.as_deref().unwrap_or("logs");
				let name = self.file_name.as_deref().unwrap_or("surrealdb.log");
				// Create the file appender based on rotation setting
				match self.file_rotation.as_deref() {
					Some("hourly") => tracing_appender::rolling::hourly(path, name),
					Some("daily") => tracing_appender::rolling::daily(path, name),
					Some("never") => tracing_appender::rolling::never(path, name),
					_ => tracing_appender::rolling::daily(path, name),
				}
			};
			// Create a non-blocking file log destination
			let (writer, guard) = NonBlockingBuilder::default()
				.lossy(false)
				.thread_name("surrealdb-logger-file")
				.finish(file_appender);
			// Get the file filter or global filter
			let (env_filter, spans_filter) = composer.with_filter(
				LoggingType::File,
				self.file_filter.clone().unwrap_or_else(|| self.filter.clone()),
			);
			// Create the file destination layer
			let layer = logs::file(env_filter, spans_filter, writer, self.file_format)?;
			// Add the layer to the registry
			layers.push(layer);
			// Add the guard to the guards
			guards.push(guard);
		}

		// Setup logging to console if enabled
		if *ENABLE_TOKIO_CONSOLE {
			// Create the console destination layer
			let layer = console::new()?;
			// Add the layer to the registry
			layers.push(layer);
		}

		if layers.is_empty() {
			// Return the registry and guards
			Ok((Box::new(registry), guards))
		} else {
			// Setup the registry layers
			let registry = registry.with(layers);
			// Return the registry and guards
			Ok((Box::new(registry), guards))
		}
	}
}

pub fn shutdown() {
	// Output information to logs
	trace!("Shutting down telemetry service");
	// Explicit shutdown is handled by Drop implementations
}

/// Create an EnvFilter from the given value. If the value is not a valid log
/// level, it will be treated as EnvFilter directives.
pub fn env_filter_from_value(v: &str) -> std::result::Result<EnvFilter, ParseError> {
	match v {
		// Don't show any logs at all
		"none" => Ok(EnvFilter::default()),
		// Otherwise, let's show only errors
		"error" => Ok(EnvFilter::default().add_directive(Level::ERROR.into())),
		// Otherwise, let's show warnings and above
		"warn" => Ok(EnvFilter::default().add_directive(Level::WARN.into())),
		// Otherwise, let's show info and above
		"info" => Ok(EnvFilter::default().add_directive(Level::INFO.into())),
		// Otherwise, let's show debugs and above
		"debug" => Ok(EnvFilter::default()
			.add_directive(Level::WARN.into())
			.add_directive("surreal=debug".parse()?)
			.add_directive("surrealdb=debug".parse()?)
			.add_directive("surrealdb::core::kvs::tx=debug".parse()?)
			.add_directive("surrealdb::core::kvs::tr=debug".parse()?)),
		// Specify the log level for each code area
		"trace" => Ok(EnvFilter::default()
			.add_directive(Level::WARN.into())
			.add_directive("surreal=trace".parse()?)
			.add_directive("surrealdb=trace".parse()?)
			.add_directive("surrealdb::core::kvs::tx=debug".parse()?)
			.add_directive("surrealdb::core::kvs::tr=debug".parse()?)),
		// Check if we should show all surreal logs
		"full" => Ok(EnvFilter::default()
			.add_directive(Level::DEBUG.into())
			.add_directive("surreal=trace".parse()?)
			.add_directive("surrealdb=trace".parse()?)
			.add_directive("surrealdb::core::kvs::tx=trace".parse()?)
			.add_directive("surrealdb::core::kvs::tr=trace".parse()?)),
		// Check if we should show all module logs
		"all" => Ok(EnvFilter::default().add_directive(Level::TRACE.into())),
		// Let's try to parse the custom log level
		_ => EnvFilter::builder().parse(v),
	}
}

/// Parse span-level directives from the given value.
///
/// Extracts entries of the form `[span_name]=level` from a comma-separated
/// filter string. Entries that do not start with `[` are silently ignored.
/// If no `=level` suffix is provided, [`LevelFilter::TRACE`] is assumed.
pub fn span_filters_from_value(v: &str) -> Vec<(String, LevelFilter)> {
	v.split(',')
		.filter_map(|d| {
			let d = d.trim();
			if !d.starts_with('[') {
				return None;
			}
			let close = d.find(']')?;
			let name = &d[1..close];
			let level = d[close + 1..].trim();
			let level = if let Some(stripped) = level.strip_prefix('=') {
				stripped.parse().ok()?
			} else {
				LevelFilter::TRACE
			};
			Some((name.to_string(), level))
		})
		.collect()
}
