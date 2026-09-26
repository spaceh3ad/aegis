use prometheus::{CounterVec, Gauge, Histogram, HistogramOpts, Opts, Registry};

#[derive(Clone)]
pub struct Metrics {
    pub transactions_total: CounterVec,
    pub transactions_blocked_total: CounterVec,
    pub inspection_duration: Histogram,
    pub blocklist_size: Gauge,
    pub downstream_up: Gauge,
    registry: Registry,
}

impl Metrics {
    pub fn new() -> Result<Self, prometheus::Error> {
        let registry = Registry::new();

        let transactions_total = CounterVec::new(
            Opts::new("aegis_transactions_total", "Total transactions by verdict"),
            &["verdict"],
        )?;
        let transactions_blocked_total = CounterVec::new(
            Opts::new("aegis_transactions_blocked_total", "Blocked transactions by rule"),
            &["rule_id"],
        )?;
        let inspection_duration = Histogram::with_opts(
            HistogramOpts::new(
                "aegis_inspection_duration_microseconds",
                "Inspection pipeline latency in microseconds",
            )
            .buckets(vec![1.0, 5.0, 10.0, 25.0, 50.0, 100.0, 250.0]),
        )?;
        let blocklist_size = Gauge::with_opts(Opts::new(
            "aegis_blocklist_size",
            "Number of entries loaded in the blocklist",
        ))?;
        let downstream_up = Gauge::with_opts(Opts::new(
            "aegis_downstream_up",
            "1 if downstream validator is reachable, 0 if not",
        ))?;

        registry.register(Box::new(transactions_total.clone()))?;
        registry.register(Box::new(transactions_blocked_total.clone()))?;
        registry.register(Box::new(inspection_duration.clone()))?;
        registry.register(Box::new(blocklist_size.clone()))?;
        registry.register(Box::new(downstream_up.clone()))?;

        Ok(Self {
            transactions_total,
            transactions_blocked_total,
            inspection_duration,
            blocklist_size,
            downstream_up,
            registry,
        })
    }

    pub fn render(&self) -> String {
        use prometheus::Encoder;
        let encoder = prometheus::TextEncoder::new();
        let mut buf = Vec::new();
        encoder.encode(&self.registry.gather(), &mut buf).unwrap_or_default();
        String::from_utf8(buf).unwrap_or_default()
    }
}
