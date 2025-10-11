use std::fs;
use std::path::PathBuf;

use blitz_dom::DocumentConfig;
use blitz_html::HtmlDocument;
use frontier::js::dom::DomState;
use url::Url;

#[test]
fn react_timer_patch_sequence_resolves() {
    let asset_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/react-demos");
    let timer_path = asset_root.join("timer.html");
    let timer_html = fs::read_to_string(&timer_path).expect("read timer.html");
    let timer_url = Url::from_file_path(&timer_path).expect("file url");

    let mut document = HtmlDocument::from_html(
        &timer_html,
        DocumentConfig {
            base_url: Some(timer_url.to_string()),
            ..Default::default()
        },
    );

    let mut dom_state = DomState::new(&timer_html);
    dom_state.attach_document(&mut document);

    let root_handle = dom_state
        .handle_from_element_id("root")
        .expect("root element handle");

    let wrapper_handle = dom_state
        .create_element("div", None)
        .expect("create wrapper div");
    dom_state
        .set_attribute_direct(&wrapper_handle, "oninput", "return;")
        .expect("set wrapper oninput");

    let _unused_container = dom_state
        .create_element("div", None)
        .expect("create inner container");

    let heading = dom_state
        .create_element("h1", None)
        .expect("create heading");
    dom_state
        .set_attribute_direct(&heading, "id", "timer-heading")
        .expect("set heading id");
    dom_state
        .set_text_content_direct(&heading, "Timer")
        .expect("set heading text");

    let value = dom_state
        .create_element("p", None)
        .expect("create value paragraph");
    dom_state
        .set_attribute_direct(&value, "id", "timer-value")
        .expect("set paragraph id");
    dom_state
        .set_text_content_direct(&value, "Elapsed: 0.0s")
        .expect("set paragraph text");

    let start_button = dom_state
        .create_element("button", None)
        .expect("create start button");
    dom_state
        .set_attribute_direct(&start_button, "id", "start-timer")
        .expect("set start id");
    dom_state
        .remove_attribute_direct(&start_button, "disabled")
        .expect("remove start disabled");
    dom_state
        .set_text_content_direct(&start_button, "Start")
        .expect("set start text");

    let stop_button = dom_state
        .create_element("button", None)
        .expect("create stop button");
    dom_state
        .set_attribute_direct(&stop_button, "id", "stop-timer")
        .expect("set stop id");
    dom_state
        .set_attribute_direct(&stop_button, "disabled", "")
        .expect("set stop disabled");
    dom_state
        .set_text_content_direct(&stop_button, "Stop")
        .expect("set stop text");

    let reset_button = dom_state
        .create_element("button", None)
        .expect("create reset button");
    dom_state
        .set_attribute_direct(&reset_button, "id", "reset-timer")
        .expect("set reset id");
    dom_state
        .set_text_content_direct(&reset_button, "Reset")
        .expect("set reset text");

    let controls = dom_state
        .create_element("div", None)
        .expect("create controls container");
    dom_state
        .append_child(&controls, &start_button)
        .expect("append start button");
    dom_state
        .append_child(&controls, &stop_button)
        .expect("append stop button");
    dom_state
        .append_child(&controls, &reset_button)
        .expect("append reset button");
    dom_state
        .set_attribute_direct(&controls, "id", "timer-controls")
        .expect("set controls id");

    let timer_root = dom_state
        .create_element("div", None)
        .expect("create timer root");
    dom_state
        .append_child(&timer_root, &heading)
        .expect("append heading");
    dom_state
        .append_child(&timer_root, &value)
        .expect("append value");
    dom_state
        .append_child(&timer_root, &controls)
        .expect("append controls");
    dom_state
        .set_attribute_direct(&timer_root, "id", "timer-root")
        .expect("set timer root id");

    dom_state
        .set_text_content_direct(&root_handle, "")
        .expect("clear root text");
    dom_state
        .append_child(&root_handle, &timer_root)
        .expect("mount timer root");

    document.resolve(0.0);
}
