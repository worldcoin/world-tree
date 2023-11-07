use metrics_exporter_statsd::StatsdBuilder;

pub fn init_statsd_exporter(service_name: &str) {
    //TODO: update these to be args
    let recorder = StatsdBuilder::from("127.0.0.1", 8125)
        .with_queue_size(5000)
        .with_buffer_size(1024)
        .build(None)
        .expect("Could not create StatsdRecorder");

    metrics::set_boxed_recorder(Box::new(recorder)).expect("TODO:");
}
