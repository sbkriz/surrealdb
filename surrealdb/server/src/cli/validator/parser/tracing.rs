use std::collections::HashMap;
use std::sync::Arc;

use clap::builder::{NonEmptyStringValueParser, PossibleValue, TypedValueParser};
use clap::error::{ContextKind, ContextValue, ErrorKind};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::filter::LevelFilter;
use tracing_subscriber::layer::{Context, Filter};
use tracing_subscriber::registry::LookupSpan;

use crate::telemetry::{env_filter_from_value, span_filters_from_value};

/// A parsed log filter configuration that combines two filtering mechanisms:
///
/// 1. An [`EnvFilter`] for module-level directive filtering (e.g. `surrealdb=debug`).
/// 2. A span-level filter map that controls log levels for specific named spans and their
///    descendants (e.g. `[my_span]=info`).
///
/// This type is produced by [`CustomFilterParser`] when parsing CLI `--log` values
/// or the `RUST_LOG` environment variable.
#[derive(Debug, Default)]
pub struct CustomFilter {
	/// The module-level environment filter parsed from the input string.
	pub(crate) env: EnvFilter,
	/// A map of span names to their maximum allowed [`LevelFilter`]. Events that
	/// occur inside a matching span (or any of its ancestor spans) are filtered
	/// according to the associated level.
	pub(crate) spans: Arc<HashMap<String, LevelFilter>>,
}

// Manual `Clone` implementation because [`EnvFilter`] does not implement `Clone`.
// We re-parse the filter from its string representation to create a copy.
impl Clone for CustomFilter {
	fn clone(&self) -> Self {
		Self {
			env: EnvFilter::builder().parse(self.env.to_string()).unwrap_or_default(),
			spans: self.spans.clone(),
		}
	}
}

impl CustomFilter {
	/// Creates a new [`CustomFilter`] from the given module-level [`EnvFilter`] and
	/// span-level filter map.
	pub fn new(env: EnvFilter, spans: Arc<HashMap<String, LevelFilter>>) -> Self {
		Self {
			env,
			spans,
		}
	}

	/// Returns a *new* [`EnvFilter`] cloned from this filter's string representation.
	///
	/// Because [`EnvFilter`] does not implement `Clone`, this re-parses the
	/// directives from the filter's `Display` output.
	pub fn env(&self) -> EnvFilter {
		EnvFilter::builder().parse(self.env.to_string()).unwrap_or_default()
	}

	/// Returns a shared reference to the span-level filter map.
	pub fn spans(&self) -> Arc<HashMap<String, LevelFilter>> {
		self.spans.clone()
	}

	/// Returns a [`SpanFilter`] for span-level filtering.
	///
	/// The returned filter checks each event against the span-level rules.
	/// If no span directives were configured, the filter will allow everything.
	pub(crate) fn span_filter(&self) -> SpanFilter {
		SpanFilter(self.spans())
	}
}

/// A per-layer filter that controls log levels for specific named spans
/// and their descendants.
///
/// An event is allowed if:
/// - Its own span name matches an entry in the map and its level is at or below the configured
///   threshold, **or**
/// - An ancestor span's name matches (the first match wins), **or**
/// - No span in the ancestry matches (the event is allowed by default).
#[derive(Clone, Debug)]
pub(crate) struct SpanFilter(Arc<HashMap<String, LevelFilter>>);

impl<S> Filter<S> for SpanFilter
where
	S: tracing::Subscriber + for<'a> LookupSpan<'a>,
{
	fn enabled(&self, meta: &tracing::Metadata<'_>, cx: &Context<'_, S>) -> bool {
		if let Some(level) = self.0.get(meta.name()) {
			return *meta.level() <= *level;
		}
		let mut current = cx.lookup_current();
		while let Some(span) = current {
			if let Some(level) = self.0.get(span.name()) {
				return *meta.level() <= *level;
			}
			current = span.parent();
		}
		true
	}
}

/// A [`TypedValueParser`] for `clap` that parses a log filter string into a
/// [`CustomFilter`].
///
/// The parser accepts either:
/// - The `RUST_LOG` environment variable (takes priority when set), or
/// - A CLI argument value.
///
/// The input string is split into two parts:
/// 1. **Module-level directives** — standard `tracing` / `EnvFilter` syntax (e.g. `info`,
///    `surrealdb=debug,warn`) parsed via
///    [`env_filter_from_value`](crate::telemetry::env_filter_from_value).
/// 2. **Span-level directives** — comma-separated entries of the form `[span_name]=level` parsed
///    via [`span_filters_from_value`](crate::telemetry::span_filters_from_value).
///
/// Recognised shorthand values: `none`, `full`, `error`, `warn`, `info`,
/// `debug`, `trace`.
#[derive(Clone)]
pub(crate) struct CustomFilterParser;

impl CustomFilterParser {
	/// Creates a new [`CustomFilterParser`].
	pub fn new() -> CustomFilterParser {
		Self
	}
}

impl TypedValueParser for CustomFilterParser {
	type Value = CustomFilter;

	fn parse_ref(
		&self,
		cmd: &clap::Command,
		arg: Option<&clap::Arg>,
		value: &std::ffi::OsStr,
	) -> Result<Self::Value, clap::Error> {
		// Fetch the log filter input
		let input = if let Ok(input) = std::env::var("RUST_LOG") {
			input
		} else {
			let inner = NonEmptyStringValueParser::new();
			inner.parse_ref(cmd, arg, value)?
		};
		// Parse the log filter input
		let env_filter = env_filter_from_value(input.as_str()).map_err(|e| {
			let mut err = clap::Error::new(ErrorKind::ValueValidation).with_cmd(cmd);
			err.insert(ContextKind::Custom, ContextValue::String(e.to_string()));
			err.insert(
				ContextKind::InvalidValue,
				ContextValue::String("Provide a valid log filter configuration string".to_string()),
			);
			err
		})?;

		let spans = span_filters_from_value(input.as_str()).into_iter().collect();
		// Return the custom targets
		Ok(CustomFilter::new(env_filter, Arc::new(spans)))
	}

	fn possible_values(&self) -> Option<Box<dyn Iterator<Item = PossibleValue> + '_>> {
		Some(Box::new(
			[
				PossibleValue::new("none"),
				PossibleValue::new("full"),
				PossibleValue::new("error"),
				PossibleValue::new("warn"),
				PossibleValue::new("info"),
				PossibleValue::new("debug"),
				PossibleValue::new("trace"),
			]
			.into_iter(),
		))
	}
}
