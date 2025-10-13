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

async fn send_keys(
    client: &Client,
    base_url: &str,
    session_id: &str,
    element_id: &str,
    text: &str,
) -> Result<()> {
    client
        .post(format!(
            "{}/session/{}/element/{}/value",
            base_url, session_id, element_id
        ))
        .json(&json!({"text": text}))
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

async fn type_keys_to_element(
    client: &Client,
    base_url: &str,
    session_id: &str,
    element_id: &str,
    text: &str,
) -> Result<()> {
    send_keys(client, base_url, session_id, element_id, text).await
}

async fn clear_element(
    client: &Client,
    base_url: &str,
    session_id: &str,
    element_id: &str,
) -> Result<()> {
    client
        .post(format!(
            "{}/session/{}/element/{}/clear",
            base_url, session_id, element_id
        ))
        .json(&json!({}))
        .send()
        .await?
        .error_for_status()?;
    Ok(())
}

async fn element_attribute(
    client: &Client,
    base_url: &str,
    session_id: &str,
    element_id: &str,
    name: &str,
) -> Result<Option<String>> {
    let response: serde_json::Value = client
        .get(format!(
            "{}/session/{}/element/{}/attribute",
            base_url, session_id, element_id
        ))
        .query(&[("name", name)])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await
        .context("parse element attribute response")?;
    Ok(response["value"].as_str().map(|s| s.to_string()))
}

async fn element_value(
    client: &Client,
    base_url: &str,
    session_id: &str,
    element_id: &str,
) -> Result<String> {
    let response: serde_json::Value = client
        .get(format!(
            "{}/session/{}/element/{}/value",
            base_url, session_id, element_id
        ))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await
        .context("parse element value response")?;
    response["value"]
        .as_str()
        .context("value missing")
        .map(|s| s.to_string())
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn webdriver_temperature_converter() -> Result<()> {
    let (handle, base_url, client) = spawn_webdriver().await?;

    let session_id = create_file_session(&client, &base_url, "temperature-converter.html").await?;
    let celsius_id = find_element(&client, &base_url, &session_id, "#celsius-input").await?;
    let fahrenheit_id = find_element(&client, &base_url, &session_id, "#fahrenheit-input").await?;
    let summary_id = find_element(&client, &base_url, &session_id, "#conversion-summary").await?;
    let status_id = find_element(&client, &base_url, &session_id, "#conversion-status").await?;

    let placeholder =
        element_attribute(&client, &base_url, &session_id, &celsius_id, "placeholder").await?;
    assert_eq!(placeholder.as_deref(), Some("0"));

    let initial_summary = element_text(&client, &base_url, &session_id, &summary_id).await?;
    assert_eq!(initial_summary, "Waiting for input.");

    let initial_status = element_text(&client, &base_url, &session_id, &status_id).await?;
    assert_eq!(initial_status, "Enter a temperature to convert.");

    click_element(&client, &base_url, &session_id, &celsius_id).await?;
    type_keys_to_element(&client, &base_url, &session_id, &celsius_id, "100").await?;
    pump_session(&client, &base_url, &session_id, 200).await?;

    let typed_celsius = element_value(&client, &base_url, &session_id, &celsius_id).await?;
    assert_eq!(typed_celsius, "100");

    let fahrenheit_value = element_value(&client, &base_url, &session_id, &fahrenheit_id).await?;
    assert_eq!(fahrenheit_value, "212.0");

    let summary_text = element_text(&client, &base_url, &session_id, &summary_id).await?;
    assert_eq!(summary_text, "Celsius 100 ↔ Fahrenheit 212.0");

    let status_text = element_text(&client, &base_url, &session_id, &status_id).await?;
    assert_eq!(status_text, "Conversion ready.");

    clear_element(&client, &base_url, &session_id, &celsius_id).await?;
    pump_session(&client, &base_url, &session_id, 100).await?;

    let cleared_celsius = element_value(&client, &base_url, &session_id, &celsius_id).await?;
    assert_eq!(cleared_celsius, "");
    let cleared_fahrenheit = element_value(&client, &base_url, &session_id, &fahrenheit_id).await?;
    assert_eq!(cleared_fahrenheit, "");

    let summary_after_clear = element_text(&client, &base_url, &session_id, &summary_id).await?;
    assert_eq!(summary_after_clear, "Waiting for input.");
    let status_after_clear = element_text(&client, &base_url, &session_id, &status_id).await?;
    assert_eq!(status_after_clear, "Enter a temperature to convert.");

    type_keys_to_element(&client, &base_url, &session_id, &celsius_id, "abc").await?;
    pump_session(&client, &base_url, &session_id, 100).await?;
    let status_invalid = element_text(&client, &base_url, &session_id, &status_id).await?;
    assert_eq!(
        status_invalid,
        "Enter a valid number to see the conversion."
    );
    let summary_invalid = element_text(&client, &base_url, &session_id, &summary_id).await?;
    assert_eq!(summary_invalid, "Input is not a number.");

    clear_element(&client, &base_url, &session_id, &celsius_id).await?;
    pump_session(&client, &base_url, &session_id, 100).await?;

    click_element(&client, &base_url, &session_id, &fahrenheit_id).await?;
    type_keys_to_element(&client, &base_url, &session_id, &fahrenheit_id, "32").await?;
    pump_session(&client, &base_url, &session_id, 200).await?;

    let typed_fahrenheit = element_value(&client, &base_url, &session_id, &fahrenheit_id).await?;
    assert_eq!(typed_fahrenheit, "32");

    let celsius_converted = element_value(&client, &base_url, &session_id, &celsius_id).await?;
    assert_eq!(celsius_converted, "0.0");

    let summary_after_fahrenheit =
        element_text(&client, &base_url, &session_id, &summary_id).await?;
    assert_eq!(summary_after_fahrenheit, "Celsius 0.0 ↔ Fahrenheit 32");
    let status_after_fahrenheit = element_text(&client, &base_url, &session_id, &status_id).await?;
    assert_eq!(status_after_fahrenheit, "Conversion ready.");

    handle.shutdown().await;
    Ok(())
}
