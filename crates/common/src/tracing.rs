use tracing::Level;
use tracing_opentelemetry::OpenTelemetrySpanExt;
use tracing_subscriber::filter::EnvFilter;
use tracing_subscriber::fmt;
use tracing_subscriber::prelude::__tracing_subscriber_SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

pub fn init_subscriber(level: Level) {
    let fmt_layer = fmt::layer().with_target(false).with_level(true);

    let filter = EnvFilter::from_default_env().add_directive(level.into());

    tracing_subscriber::registry()
        .with(filter)
        .with(fmt_layer)
        .init();
}
pub fn init_datadog_subscriber(service_name: &str, level: Level) {
    let tracer = opentelemetry_datadog::new_pipeline()
        .with_service_name(service_name)
        .with_api_version(opentelemetry_datadog::ApiVersion::Version05)
        .install_simple()
        .expect("Could not initialize tracer");

    let otel_layer = tracing_opentelemetry::OpenTelemetryLayer::new(tracer);

    let filter = tracing_subscriber::filter::EnvFilter::from_default_env()
        .add_directive(level.into());

    tracing_subscriber::registry()
        .with(filter)
        .with(otel_layer)
        .init();
}

/// Extract the W3C Trace Context from the headers of a request and add them
/// to the current span.
pub fn trace_from_headers(headers: &http::HeaderMap) {
    tracing::Span::current().set_parent(
        opentelemetry::global::get_text_map_propagator(|propagator| {
            propagator.extract(&opentelemetry_http::HeaderExtractor(headers))
        }),
    );
}

pub fn trace_to_headers(headers: &mut http::HeaderMap) {
    opentelemetry::global::get_text_map_propagator(|propagator| {
        propagator.inject_context(
            &tracing::Span::current().context(),
            &mut opentelemetry_http::HeaderInjector(headers),
        );
    });
}
