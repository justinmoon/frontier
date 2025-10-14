use std::path::PathBuf;
use std::time::Duration;

use anyhow::{anyhow, Result};
use frontier::automation_client::{
    AutomationHost, AutomationHostConfig, ElementSelector, WaitOptions,
};
use url::Url;

#[test]
#[ignore = "Back button regression: clicking #back-button does not restore previous content"]
fn back_button_regression_reproduction() -> Result<()> {
    let asset_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/react-demos");
    let index_path = asset_root.join("index.html");
    let timer_path = asset_root.join("timer.html");
    let timer_url = Url::from_file_path(&timer_path)
        .map_err(|_| anyhow!("unable to form file:// timer url"))?;

    let initial_target = Url::from_file_path(&index_path)
        .map_err(|_| anyhow!("unable to form file:// initial url"))?;

    let host = AutomationHost::spawn(
        AutomationHostConfig::default()
            .with_asset_root(asset_root.clone())
            .with_initial_target(initial_target.as_str().to_string()),
    )?;

    let session = host.session_from_asset("index.html")?;

    let initial_content = session.wait_for_text(
        &ElementSelector::css("#content"),
        WaitOptions::new(Duration::from_secs(3), Duration::from_millis(250)),
    )?;
    assert!(
        initial_content.contains("Frontier React Demos"),
        "expected index content after initial load, found {initial_content:?}"
    );

    session.type_text(&ElementSelector::css("#url-input"), timer_url.as_str())?;
    session.click(&ElementSelector::css("#go-button"))?;
    session.navigate_url(timer_url.as_str())?;

    let timer_heading = session.wait_for_text(
        &ElementSelector::css("#timer-heading"),
        WaitOptions::new(Duration::from_secs(5), Duration::from_millis(250)),
    )?;
    assert!(
        timer_heading.contains("Timer"),
        "expected timer heading after navigation, but saw {timer_heading:?}"
    );

    session.click(&ElementSelector::css("#back-button"))?;
    let restored_content = session.wait_for_text(
        &ElementSelector::css("#content"),
        WaitOptions::new(Duration::from_secs(5), Duration::from_millis(250)),
    )?;

    assert!(
        restored_content.contains("Frontier React Demos"),
        "expected back-button to restore index page content, saw {restored_content:?}"
    );

    Ok(())
}
