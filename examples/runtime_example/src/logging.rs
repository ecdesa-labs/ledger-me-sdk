//! Development logging: `tracing-subscriber`'s pretty format, plus the
//! OpenTelemetry trace ID of the call each event belongs to.
//!
//! The SDK only emits spans; the trace ID comes from the OpenTelemetry layer
//! installed here, which gives every span tree one `trace_id`. Nothing is
//! exported (no Jaeger/OTLP yet): the IDs only show up in the terminal.

use std::fmt;
use std::sync::OnceLock;

use opentelemetry::trace::{TraceContextExt, TraceId, TracerProvider};
use opentelemetry_sdk::trace::SdkTracerProvider;
use tracing::dispatcher::{Dispatch, WeakDispatch};
use tracing::{Event, Subscriber};
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::EnvFilter;

/// The installed subscriber, which `tracing_opentelemetry::get_otel_context`
/// needs to look a span's trace ID up. Weak, so it doesn't keep the
/// subscriber alive by itself.
static SUBSCRIBER: OnceLock<WeakDispatch> = OnceLock::new();

/// Installs the global subscriber. `RUST_LOG` overrides the default filter
/// (e.g. `RUST_LOG=warn` to only see failures).
pub fn init() -> anyhow::Result<()> {
    let tracer = SdkTracerProvider::builder().build().tracer("runtime_example");
    let default_filter = EnvFilter::new("info,ledger_me_sdk=debug");

    let subscriber = tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or(default_filter))
        .with(tracing_opentelemetry::layer().with_tracer(tracer))
        .with(
            tracing_subscriber::fmt::layer()
                .pretty()
                .event_format(WithTraceId(tracing_subscriber::fmt::format().pretty())),
        );

    let subscriber = Dispatch::new(subscriber);
    let _ = SUBSCRIBER.set(subscriber.downgrade());
    tracing::dispatcher::set_global_default(subscriber)?;

    Ok(())
}

/// Wraps an event format, writing a `trace_id: ...` line above each event
/// that happens inside a span.
struct WithTraceId<F>(F);

impl<S, N, F> FormatEvent<S, N> for WithTraceId<F>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
    F: FormatEvent<S, N>,
{
    fn format_event(&self, ctx: &FmtContext<'_, S, N>, mut writer: Writer<'_>, event: &Event<'_>) -> fmt::Result {
        if let Some(trace_id) = ctx.parent_span().and_then(|span| trace_id_of(&span.id())) {
            if writer.has_ansi_escapes() {
                writeln!(writer, "  \x1b[2mtrace_id: {trace_id}\x1b[0m")?;
            } else {
                writeln!(writer, "  trace_id: {trace_id}")?;
            }
        }

        self.0.format_event(ctx, writer, event)
    }
}

fn trace_id_of(span: &tracing::span::Id) -> Option<TraceId> {
    let subscriber = SUBSCRIBER.get()?.upgrade()?;
    let otel_context = tracing_opentelemetry::get_otel_context(span, &subscriber)?;

    Some(otel_context.span().span_context().trace_id())
}
