//! GUI automation harness that reproduces the React navigation crash by
//! launching the full Frontier binary and interacting the same way a user does.
//!
//! These tests are `#[ignore]` because they require a running GUI session and
//! will steal focus. Run them manually with:
//!   cargo test --test gui_automation_test -- --ignored --test-threads=1 --nocapture

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use enigo::{Button, Coordinate, Direction, Enigo, Mouse, Settings};
use tokio::io::AsyncReadExt;
use tokio::time::sleep;

fn build_frontier() -> PathBuf {
    let status = std::process::Command::new("cargo")
        .args(["build", "--bin", "frontier"])
        .status()
        .expect("build frontier binary");
    assert!(status.success(), "frontier build failed");

    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("debug")
        .join(if cfg!(windows) {
            "frontier.exe"
        } else {
            "frontier"
        })
}

async fn launch_frontier(url: &str) -> tokio::process::Child {
    tokio::process::Command::new(build_frontier())
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn frontier binary")
}

async fn bring_window_to_front() {
    #[cfg(target_os = "macos")]
    {
        let _ = tokio::process::Command::new("osascript")
            .args([
                "-e",
                r#"tell application "System Events"
                    set frontierProcess to first process whose name contains "frontier"
                    set frontmost of frontierProcess to true
                end tell"#,
            ])
            .output()
            .await;
    }
}

fn click_at(x: i32, y: i32) {
    let mut enigo = Enigo::new(&Settings::default()).expect("create enigo");
    enigo.move_mouse(x, y, Coordinate::Abs).expect("move mouse");
    std::thread::sleep(Duration::from_millis(100));
    enigo.button(Button::Left, Direction::Press).expect("press");
    std::thread::sleep(Duration::from_millis(50));
    enigo
        .button(Button::Left, Direction::Release)
        .expect("release");
}

async fn consume_stderr(child: &mut tokio::process::Child) -> String {
    if let Some(mut stderr) = child.stderr.take() {
        let mut buf = Vec::new();
        let _ = stderr.read_to_end(&mut buf).await;
        String::from_utf8_lossy(&buf).into_owned()
    } else {
        String::new()
    }
}

#[tokio::test]
#[ignore = "Launches a GUI session and intentionally reproduces the React navigation crash"]
async fn react_navigation_crash_regression() {
    let demos = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/react-demos");
    let url = format!("file://{}", demos.join("test-index.html").display());

    let mut app = launch_frontier(&url).await;
    sleep(Duration::from_secs(3)).await;
    bring_window_to_front().await;
    sleep(Duration::from_millis(500)).await;

    // Click the full-window link – the coordinates roughly match the center of the window.
    click_at(640, 360);
    sleep(Duration::from_secs(2)).await;

    let status = match app.try_wait() {
        Ok(Some(status)) => status,
        Ok(None) => {
            // App survived; treat as a failure because we expect the crash today.
            let _ = app.kill().await;
            panic!("Frontier stayed alive; crash no longer reproduces");
        }
        Err(err) => panic!("Failed to query process status: {err}"),
    };

    assert!(
        !status.success(),
        "Expected crash exit status capturing the stylo panic"
    );

    let stderr_output = consume_stderr(&mut app).await;
    assert!(
        stderr_output.contains("DOM mutation failed")
            || stderr_output.contains("panic")
            || stderr_output.contains("stylo"),
        "stderr should contain crash diagnostics, got: {}",
        stderr_output
    );
}
