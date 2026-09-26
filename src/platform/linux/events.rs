//! Calendar events. There is no portable Linux calendar API equivalent to
//! macOS EventKit, so this is a stub that returns no events. The `Events`
//! main-page option therefore renders empty on Linux.

use crate::app::{
    ToApp,
    apps::{App, AppCommand, ICNS_ICON},
};
use crate::utils::icns_data_to_handle;

#[derive(Debug, Clone, PartialEq)]
pub struct Event {
    pub event_name: String,
    pub event_url: Option<String>,
    pub time: String,
}

impl ToApp for Event {
    fn to_app(&self) -> App {
        let icons = icns_data_to_handle(ICNS_ICON.to_vec());
        let appcmd = match &self.event_url {
            Some(url) => AppCommand::Function(crate::commands::Function::OpenRawUrl(url.clone())),
            None => AppCommand::Display,
        };
        App::new(self.event_name.clone(), icons, self.time.clone(), appcmd)
    }
}

impl Event {
    pub fn get_events(_duration_in_min: u32) -> Vec<Self> {
        Vec::new()
    }
}
