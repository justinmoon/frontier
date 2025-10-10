use std::path::PathBuf;
use std::time::Duration;

use frontier::HeadlessSessionBuilder;
use tokio::runtime::Builder;

fn asset_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/react-demos")
}

#[test]
fn headless_counter_increment() {
    let runtime = Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let mut session = HeadlessSessionBuilder::new()
            .with_base_dir(asset_root())
            .open_file("counter.html")
            .await
            .expect("open counter");

        assert_eq!(session.inner_text("#counter-value").unwrap(), "Count: 0");
        session.click("#increment").await.unwrap();
        session.pump_for(Duration::from_millis(50)).await;
        assert_eq!(session.inner_text("#counter-value").unwrap(), "Count: 1");
    });
}

#[test]
fn headless_timer_start_stop() {
    let runtime = Builder::new_current_thread().enable_all().build().unwrap();
    runtime.block_on(async {
        let mut session = HeadlessSessionBuilder::new()
            .with_base_dir(asset_root())
            .open_file("timer.html")
            .await
            .expect("open timer");

        assert_eq!(session.inner_text("#timer-value").unwrap(), "Elapsed: 0.0s");
        session.click("#start-timer").await.unwrap();
        session.pump_for(Duration::from_millis(400)).await;
        let running = session.inner_text("#timer-value").unwrap();
        assert_ne!(running, "Elapsed: 0.0s");
        session.click("#stop-timer").await.unwrap();
        let stopped = session.inner_text("#timer-value").unwrap();
        session.pump_for(Duration::from_millis(400)).await;
        let after = session.inner_text("#timer-value").unwrap();
        assert_eq!(stopped, after, "timer should freeze after stop");
    });
}
