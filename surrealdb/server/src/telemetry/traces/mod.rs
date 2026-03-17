pub mod rpc;

use anyhow::Result;
use opentelemetry::trace::TracerProvider as _;
use opentelemetry_sdk::trace::SdkTracerProvider;
use tracing::Subscriber;
use tracing_subscriber::layer::Filter;
use tracing_subscriber::{EnvFilter, Layer};

use crate::cnf::{TELEMETRY_DISABLE_TRACING, TELEMETRY_PROVIDER};
use crate::telemetry::OTEL_DEFAULT_RESOURCE;

/// Create an OpenTelemetry tracing layer if the `SURREAL_TELEMETRY_PROVIDER`
/// environment variable is set to `"otlp"` and tracing is not disabled.
///
/// The returned layer exports spans via the OTLP/gRPC protocol. Both the
/// `env_filter` (module-level directives) and `span_filter` (span-level rules)
/// are applied as per-layer filters so they do not affect other layers in the
/// subscriber stack.
///
/// Returns `Ok(None)` when telemetry tracing is not configured.
pub fn new<F, S>(
	env_filter: EnvFilter,
	span_filter: F,
) -> Result<Option<Box<dyn Layer<S> + Send + Sync>>>
where
	F: Filter<S> + Send + Sync + 'static,
	S: Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a> + Send + Sync,
{
	match TELEMETRY_PROVIDER.trim() {
		// The OTLP telemetry provider has been specified
		s if s.eq_ignore_ascii_case("otlp") && !*TELEMETRY_DISABLE_TRACING => {
			// Build a new span exporter which uses gRPC via tonic
			let span_exporter = opentelemetry_otlp::SpanExporter::builder().with_tonic().build()?;
			// Create a batch span processor with the exporter (uses Tokio runtime automatically)
			let batch_processor =
				opentelemetry_sdk::trace::BatchSpanProcessor::builder(span_exporter).build();
			// Create the provider
			let provider = SdkTracerProvider::builder()
				.with_span_processor(batch_processor)
				.with_resource(OTEL_DEFAULT_RESOURCE.clone())
				.build();
			// Set it as the global tracer provider
			opentelemetry::global::set_tracer_provider(provider.clone());
			// Return the tracing layer with the specified filter
			Ok(Some(
				tracing_opentelemetry::layer()
					.with_tracer(provider.tracer("surealdb"))
					.with_filter(env_filter)
					.with_filter(span_filter)
					.boxed(),
			))
		}
		// No matching telemetry provider was found
		_ => Ok(None),
	}
}
