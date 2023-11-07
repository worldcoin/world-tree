use metrics_exporter_statsd::StatsdBuilder;

pub fn init_statsd_exporter(host: &str, port: u16) {
    //TODO: update these to be args
    let recorder = StatsdBuilder::from(host, port)
        .with_queue_size(5000)
        .with_buffer_size(1024)
        .build(None)
        .expect("Could not create StatsdRecorder");

    metrics::set_boxed_recorder(Box::new(recorder)).expect("TODO:");
}
