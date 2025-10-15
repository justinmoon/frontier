use std::path::PathBuf;
use std::time::Duration;

use anyhow::Result;
use frontier::automation_client::{
    AutomationHost, AutomationHostConfig, ElementSelector, WaitOptions,
};

fn asset_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/react-demos")
}

fn default_wait() -> WaitOptions {
    WaitOptions::new(Duration::from_secs(2), Duration::from_millis(100))
}

#[test]
fn automation_counter_increment() -> Result<()> {
    let asset_root = asset_root();
    let host =
        AutomationHost::spawn(AutomationHostConfig::default().with_asset_root(asset_root.clone()))?;

    let session = host.session_from_asset("counter.html")?;

    let counter_selector = ElementSelector::css("#counter-value");
    let increment_selector = ElementSelector::css("#increment");

    let initial = session.wait_for_text(&counter_selector, default_wait())?;
    assert_eq!(initial.trim(), "Count: 0");

    session.click(&increment_selector)?;
    session.pump(Duration::from_millis(50))?;

    let after = session.wait_for_text(&counter_selector, default_wait())?;
    assert_eq!(after.trim(), "Count: 1");

    Ok(())
}

#[test]
fn automation_timer_start_stop() -> Result<()> {
    let asset_root = asset_root();
    let host =
        AutomationHost::spawn(AutomationHostConfig::default().with_asset_root(asset_root.clone()))?;

    let session = host.session_from_asset("timer.html")?;

    let timer_selector = ElementSelector::css("#timer-value");
    let start_selector = ElementSelector::css("#start-timer");
    let stop_selector = ElementSelector::css("#stop-timer");

    let initial = session.wait_for_text(&timer_selector, default_wait())?;
    assert_eq!(initial.trim(), "Elapsed: 0.0s");

    session.click(&start_selector)?;
    session.pump(Duration::from_millis(400))?;

    let running = session.wait_for_text(&timer_selector, default_wait())?;
    assert_ne!(running.trim(), "Elapsed: 0.0s");

    session.click(&stop_selector)?;
    session.pump(Duration::from_millis(400))?;

    let stopped = session.wait_for_text(&timer_selector, default_wait())?;
    session.pump(Duration::from_millis(400))?;
    let after = session.wait_for_text(&timer_selector, default_wait())?;

    assert_eq!(
        stopped.trim(),
        after.trim(),
        "timer should freeze after stop"
    );

    Ok(())
}
