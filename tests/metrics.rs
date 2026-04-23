use npm_docker_sync::config::MetricsConfig;
use std::time::Duration;

#[tokio::test(flavor = "multi_thread")]
async fn init_disabled_returns_none() {
    let cfg = MetricsConfig {
        enabled: false,
        ..MetricsConfig::default()
    };
    let guard = npm_docker_sync::metrics::init(&cfg).expect("init");
    assert!(guard.is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn init_enabled_exposes_scrape_endpoint() {
    // Probe an ephemeral port by binding + immediately dropping.
    let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);

    let cfg = MetricsConfig {
        enabled: true,
        bind: format!("127.0.0.1:{port}").parse().unwrap(),
    };

    // Guard must live until the end of the test.
    let _guard = npm_docker_sync::metrics::init(&cfg).expect("init");
    assert!(_guard.is_some());

    // Emit one counter value so the scrape isn't completely empty.
    metrics::counter!("npm_docker_sync_test_counter").increment(1);

    // Give the exporter a moment to record and expose it.
    tokio::time::sleep(Duration::from_millis(200)).await;

    let body = reqwest::get(format!("http://127.0.0.1:{port}/metrics"))
        .await
        .expect("scrape request")
        .text()
        .await
        .expect("scrape body");
    assert!(
        body.contains("npm_docker_sync_test_counter"),
        "metric not in scrape body: {body}"
    );
}
