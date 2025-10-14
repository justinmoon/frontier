use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use anyhow::{anyhow, Context, Result};
use reqwest::blocking::Client;
use serde_json::json;
use url::Url;

fn asset_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/react-demos")
}

#[test]
#[ignore = "Back button regression: clicking #back-button does not restore previous content"]
fn back_button_regression_reproduction() -> Result<()> {
    let root = asset_root();
    let index_path = root.join("index.html");
    let timer_path = root.join("timer.html");
    let timer_url = Url::from_file_path(&timer_path)
        .map_err(|_| anyhow!("unable to form file:// timer url"))?;

    let (mut host, addr) = launch_host(&root, &index_path)?;
    let client = Client::new();

    let outcome: Result<String> = (|| {
        create_session(&client, &addr, "index.html")?;

        let index_text = wait_for_text(&client, &addr, "#content", 12, Duration::from_millis(250))?;
        assert!(
            index_text.contains("Frontier React Demos"),
            "expected index content after initial load, found {index_text:?}"
        );

        type_text(&client, &addr, "#url-input", timer_url.as_str())?;
        click(&client, &addr, "#go-button")?;
        navigate_url(&client, &addr, timer_url.as_str())?;

        let timer_heading = wait_for_text(
            &client,
            &addr,
            "#timer-heading",
            20,
            Duration::from_millis(250),
        )?;
        assert!(
            timer_heading.contains("Timer"),
            "expected timer heading after navigation, but saw {timer_heading:?}"
        );

        click(&client, &addr, "#back-button")?;
        wait_for_text(&client, &addr, "#content", 20, Duration::from_millis(250))
    })();

    host.shutdown();

    let content_text = outcome?;

    assert!(
        content_text.contains("Frontier React Demos"),
        "expected back-button to restore index page content, saw {content_text:?}"
    );

    Ok(())
}

struct HostHandle {
    child: std::process::Child,
    _stdout: BufReader<std::process::ChildStdout>,
}

impl HostHandle {
    fn shutdown(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn launch_host(asset_root: &Path, index_path: &Path) -> Result<(HostHandle, String)> {
    let initial_url = Url::from_file_path(index_path)
        .map_err(|_| anyhow!("unable to form file:// initial url"))?;
    let mut child = Command::new(env!("CARGO_BIN_EXE_automation_host"))
        .env("AUTOMATION_ASSET_ROOT", asset_root.display().to_string())
        .env("AUTOMATION_BIND", "127.0.0.1:0")
        .env("AUTOMATION_INITIAL", initial_url.as_str())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .context("spawn automation host")?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("automation host stdout unavailable"))?;
    let mut reader = BufReader::new(stdout);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .context("read automation host banner")?;
    let line = line.trim();
    let addr = line
        .strip_prefix("AUTOMATION_HOST_READY ")
        .ok_or_else(|| anyhow!("unexpected host banner: {line}"))?
        .to_string();
    Ok((
        HostHandle {
            child,
            _stdout: reader,
        },
        addr,
    ))
}

fn create_session(client: &Client, addr: &str, file: &str) -> Result<()> {
    client
        .post(format!("http://{addr}/session"))
        .json(&json!({"file": file}))
        .send()
        .context("create session request")?
        .error_for_status()
        .context("create session response")?;
    Ok(())
}

fn click(client: &Client, addr: &str, selector: &str) -> Result<()> {
    client
        .post(format!("http://{addr}/session/frontier/click"))
        .json(&json!({"selector": selector}))
        .send()
        .context("click request")?
        .error_for_status()
        .context("click response")?;
    Ok(())
}

fn type_text(client: &Client, addr: &str, selector: &str, text: &str) -> Result<()> {
    client
        .post(format!("http://{addr}/session/frontier/type"))
        .json(&json!({"selector": selector, "text": text}))
        .send()
        .context("type request")?
        .error_for_status()
        .context("type response")?;
    Ok(())
}

fn pump(client: &Client, addr: &str, duration: Duration) -> Result<()> {
    client
        .post(format!("http://{addr}/session/frontier/pump"))
        .json(&json!({"milliseconds": duration.as_millis() as u64}))
        .send()
        .context("pump request")?
        .error_for_status()
        .context("pump response")?;
    Ok(())
}

fn navigate_url(client: &Client, addr: &str, url: &str) -> Result<()> {
    client
        .post(format!("http://{addr}/session/frontier/navigate"))
        .json(&json!({"url": url}))
        .send()
        .context("navigate request")?
        .error_for_status()
        .context("navigate response")?;
    Ok(())
}

fn wait_for_text(
    client: &Client,
    addr: &str,
    selector: &str,
    attempts: usize,
    pump_interval: Duration,
) -> Result<String> {
    let encoded = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("selector", selector)
        .finish();
    let mut last_status = None;
    let mut last_body = String::new();
    for _ in 0..attempts {
        let response = client
            .get(format!("http://{addr}/session/frontier/text?{encoded}"))
            .send()
            .context("get text request")?;

        if response.status().is_success() {
            let body: serde_json::Value = response.json().context("parse text response body")?;
            let value = body["value"]
                .as_str()
                .ok_or_else(|| anyhow!("text response missing value field"))?
                .to_string();
            return Ok(value);
        } else {
            last_status = Some(response.status());
            last_body = response
                .text()
                .unwrap_or_else(|_| "<unavailable>".to_string());
            pump(client, addr, pump_interval)?;
        }
    }

    Err(anyhow!(
        "selector {selector} unavailable after {attempts} attempts (last_status: {:?}, body: {})",
        last_status,
        last_body
    ))
}
