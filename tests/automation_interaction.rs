use std::path::PathBuf;
use std::time::Duration;

use anyhow::{anyhow, Result};
use frontier::automation_client::{
    AutomationHost, AutomationHostConfig, ElementSelector, KeyboardAction, PointerAction,
    PointerButton, PointerTarget, WaitOptions,
};
use url::Url;

#[test]
fn automation_form_interaction() -> Result<()> {
    let asset_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/automation");
    let form_path = asset_root.join("form.html");
    let form_url = Url::from_file_path(&form_path)
        .map_err(|_| anyhow!("unable to form file:// url for automation form"))?;

    let host = AutomationHost::spawn(
        AutomationHostConfig::default()
            .with_asset_root(asset_root)
            .with_initial_target(form_url.as_str().to_string()),
    )?;

    let session = host.session_from_asset("form.html")?;

    let title_selector = ElementSelector::css("#title");
    session.wait_for_text(&title_selector, WaitOptions::default_text_wait())?;

    let name_selector = ElementSelector::css("#name-input");
    let button_selector = ElementSelector::Role {
        role: "button".into(),
        name: Some("Submit form".into()),
    };

    session.scroll_into_view(&button_selector)?;
    session.focus(&name_selector)?;
    session.type_text(&name_selector, "Ada")?;
    session.keyboard_sequence(vec![KeyboardAction::Text {
        value: " Lovelace".into(),
    }])?;

    session.pointer_sequence(vec![
        PointerAction::Move {
            to: PointerTarget::Element {
                selector: button_selector.clone(),
                offset: None,
            },
        },
        PointerAction::Down {
            button: PointerButton::Primary,
        },
        PointerAction::Pause { duration_ms: 24 },
        PointerAction::Up {
            button: PointerButton::Primary,
        },
    ])?;

    let status_selector = ElementSelector::css("#status");
    let status_text = session.wait_for_text(&status_selector, WaitOptions::default_text_wait())?;
    assert!(
        status_text.starts_with("Hello"),
        "status should update after submission (got {status_text:?})"
    );

    session.wait_for_element(
        &ElementSelector::Role {
            role: "status".into(),
            name: Some("Submitted".into()),
        },
        WaitOptions::new(Duration::from_secs(2), Duration::from_millis(100)),
    )?;

    Ok(())
}
