// Library exports for integration tests, automation harnesses, and WebDriver glue.

#[cfg(feature = "gpu")]
pub use anyrender_vello::VelloWindowRenderer as WindowRenderer;
#[cfg(feature = "cpu-base")]
pub use anyrender_vello_cpu::VelloCpuWindowRenderer as WindowRenderer;

pub use blitz_shell::{create_default_event_loop, WindowConfig};

pub mod automation;
pub mod chrome;
pub mod input;
pub mod js;
pub mod navigation;
pub mod readme_application;
pub mod webdriver;
pub mod wpt;

pub use automation::{HeadlessSession, HeadlessSessionBuilder};
pub use chrome::wrap_with_url_bar;
pub use readme_application::{NavigationMessage, ReadmeApplication};
pub use webdriver::{start_webdriver, WebDriverConfig, WebDriverHandle};
