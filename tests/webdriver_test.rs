use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::{Context, Result};
use frontier::{start_webdriver, WebDriverConfig, WebDriverHandle};
use reqwest::Client;
use serde_json::json;
use tokio::time::sleep;

fn asset_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/react-demos")
}

async fn spawn_webdriver() -> Result<(WebDriverHandle, String, Client)> {
    let config = WebDriverConfig {
        asset_root: asset_root(),
    };
    let handle = start_webdriver(SocketAddr::from(([127, 0, 0, 1], 0)), config).await?;
    let base_url = format!("http://{}", handle.addr);
    Ok((handle, base_url, Client::new()))
}

async fn create_file_session(client: &Client, base_url: &str, file: &str) -> Result<String> {
    let response: serde_json::Value = client
        .post(format!("{}/session", base_url))
        .json(&json!({"file": file}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await
        .with_context(|| format!("parse create session response for {}", file))?;
    response["value"]["sessionId"]
        .as_str()
        .context("session id missing")
        .map(|s| s.to_string())
}

async fn get_session_url(client: &Client, base_url: &str, session_id: &str) -> Result<String> {
    let response: serde_json::Value = client
        .get(format!("{}/session/{}/url", base_url, session_id))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await
        .context("parse get url response")?;
    response["value"]
        .as_str()
        .context("url missing")
        .map(|s| s.to_string())
}

async fn get_session_source(client: &Client, base_url: &str, session_id: &str) -> Result<String> {
    let response: serde_json::Value = client
        .get(format!("{}/session/{}/source", base_url, session_id))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await
        .context("parse get source response")?;
    response["value"]
        .as_str()
        .context("source missing")
        .map(|s| s.to_string())
}

async fn find_element(
    client: &Client,
    base_url: &str,
    session_id: &str,
    selector: &str,
) -> Result<String> {
    let response: serde_json::Value = client
        .post(format!("{}/session/{}/element", base_url, session_id))
        .json(&json!({"using": "css selector", "value": selector}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await
        .with_context(|| format!("parse find element response for {}", selector))?;
    response["value"]["element-6066-11e4-a52e-4f735466cecf"]
        .as_str()
        .context("element id missing")
        .map(|s| s.to_string())
}

async fn element_text(
    client: &Client,
    base_url: &str,
    session_id: &str,
    element_id: &str,
) -> Result<String> {
    let response: serde_json::Value = client
        .post(format!(
            "{}/session/{}/element/{}/text",
            base_url, session_id, element_id
        ))
        .json(&json!({}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await
        .context("parse element text response")?;
    response["value"]
        .as_str()
        .context("text missing")
        .map(|s| s.to_string())
}

async fn click_element(
    client: &Client,
    base_url: &str,
    session_id: &str,
    element_id: &str,
) -> Result<()> {
    client
        .post(format!(
            "{}/session/{}/element/{}/click",
            base_url, session_id, element_id
        ))
        .json(&json!({}))
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

fn parse_elapsed_seconds(text: &str) -> Result<f32> {
    let numeric = text
        .strip_prefix("Elapsed: ")
        .context("timer text missing prefix")?
        .strip_suffix("s")
        .context("timer text missing suffix")?;
    numeric.parse::<f32>().context("timer elapsed not a float")
}

async fn pump_session(
    client: &Client,
    base_url: &str,
    session_id: &str,
    milliseconds: u64,
) -> Result<()> {
    client
        .post(format!("{}/session/{}/frontier/pump", base_url, session_id))
        .json(&json!({"milliseconds": milliseconds}))
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn webdriver_counter_click() -> Result<()> {
    let (handle, base_url, client) = spawn_webdriver().await?;

    let session_id = create_file_session(&client, &base_url, "counter.html").await?;

    let current_url = get_session_url(&client, &base_url, &session_id).await?;
    assert!(current_url.ends_with("counter.html"));

    let source = get_session_source(&client, &base_url, &session_id).await?;
    assert!(source.contains("id=\"counter-value\""));

    let increment_id = find_element(&client, &base_url, &session_id, "#increment").await?;
    let counter_id = find_element(&client, &base_url, &session_id, "#counter-value").await?;

    let initial_text = element_text(&client, &base_url, &session_id, &counter_id).await?;
    assert_eq!(initial_text, "Count: 0");

    click_element(&client, &base_url, &session_id, &increment_id).await?;

    sleep(Duration::from_millis(50)).await;

    let after_click = element_text(&client, &base_url, &session_id, &counter_id).await?;
    assert_eq!(after_click, "Count: 1");

    handle.shutdown().await;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn webdriver_timer_start_stop() -> Result<()> {
    let (handle, base_url, client) = spawn_webdriver().await?;

    let session_id = create_file_session(&client, &base_url, "timer.html").await?;

    let timer_value_id = find_element(&client, &base_url, &session_id, "#timer-value").await?;
    let start_button_id = find_element(&client, &base_url, &session_id, "#start-timer").await?;
    let stop_button_id = find_element(&client, &base_url, &session_id, "#stop-timer").await?;

    let initial_text = element_text(&client, &base_url, &session_id, &timer_value_id).await?;
    assert_eq!(initial_text, "Elapsed: 0.0s");

    click_element(&client, &base_url, &session_id, &start_button_id).await?;

    // Pump for 1 second (1000ms) to let timer advance
    pump_session(&client, &base_url, &session_id, 1000).await?;

    let running_text = element_text(&client, &base_url, &session_id, &timer_value_id).await?;
    let running_seconds = parse_elapsed_seconds(&running_text)?;

    println!(
        "After clicking start and waiting 1 second: {}",
        running_text
    );
    println!("Parsed seconds: {}", running_seconds);

    assert!(
        running_seconds > 0.0,
        "timer should advance after being started, but got: {} ({}s)",
        running_text,
        running_seconds
    );

    // Timer should be close to 1 second (allowing some margin)
    assert!(
        (0.8..=1.2).contains(&running_seconds),
        "timer should be around 1.0s after 1 second, but got: {} ({}s)",
        running_text,
        running_seconds
    );

    click_element(&client, &base_url, &session_id, &stop_button_id).await?;
    pump_session(&client, &base_url, &session_id, 100).await?;

    let stopped_text = element_text(&client, &base_url, &session_id, &timer_value_id).await?;
    let stopped_seconds = parse_elapsed_seconds(&stopped_text)?;

    pump_session(&client, &base_url, &session_id, 500).await?;
    let after_stop_text = element_text(&client, &base_url, &session_id, &timer_value_id).await?;
    let after_stop_seconds = parse_elapsed_seconds(&after_stop_text)?;
    assert!(
        (after_stop_seconds - stopped_seconds).abs() < 0.05,
        "timer should remain paused after stop (was {stopped_text}, later {after_stop_text})"
    );

    handle.shutdown().await;
    Ok(())
}
