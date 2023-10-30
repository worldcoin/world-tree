use opentelemetry::global;
use opentelemetry::sdk::propagation::TraceContextPropagator;
use opentelemetry::sdk::trace::{BatchSpanProcessor, Sampler, Tracer};
use opentelemetry::trace::TracerProvider;
use opentelemetry_datadog::DatadogExporter;
use tracing::{Level, Subscriber};
use tracing_opentelemetry::OpenTelemetryLayer;
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
    //     let exporter = DatadogExporter::builder()
    //         .with_service_name(service_name)
    //         .init()
    //         .expect("Error initializing Datadog exporter");

    //     let batch = BatchSpanProcessor::builder(
    //         exporter,
    //         tokio::runtime::Handle::current(),
    //     )
    //     .build();

    //     let provider = opentelemetry::sdk::trace::TracerProvider::builder()
    //         .with_simple_exporter(batch)
    //         .with_config(opentelemetry::sdk::trace::Config {
    //             sampler: Box::new(Sampler::AlwaysOn),
    //             ..Default::default()
    //         })
    //         .build();

    //     let tracer = provider.tracer("my_tracer");
    //     let otel_layer = OpenTelemetryLayer::new(tracer);

    //     let filter = EnvFilter::from_default_env().add_directive(level.into());

    //     global::set_text_map_propagator(TraceContextPropagator::new());

    //     tracing_subscriber::registry()
    //         .with(filter)
    //         .with(otel_layer)
    //         .init();
}
