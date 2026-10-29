//! The tray icon (StatusNotifierItem): the unread dot, and a menu for opening, composing, checking
//! now, do-not-disturb and quitting.

use std::sync::Arc;

use ksni::menu::{CheckmarkItem, StandardItem};
use ksni::{Icon, MenuItem, ToolTip, TrayMethods};

use super::Hub;
use crate::icon;
use crate::ipc::Event;

pub struct PostieTray {
    pub unread: i64,
    pub dnd: bool,
    hub: Arc<Hub>,
}

pub async fn spawn(hub: Arc<Hub>, dnd: bool) -> Result<ksni::Handle<PostieTray>, ksni::Error> {
    PostieTray { unread: 0, dnd, hub }.spawn().await
}

impl ksni::Tray for PostieTray {
    fn id(&self) -> String {
        "postie".into()
    }

    fn title(&self) -> String {
        "Postie".into()
    }

    fn icon_pixmap(&self) -> Vec<Icon> {
        [22, 32, 48, 64]
            .into_iter()
            .map(|size| Icon { width: size as i32, height: size as i32, data: icon::argb(size, self.unread > 0) })
            .collect()
    }

    fn tool_tip(&self) -> ToolTip {
        let description = match self.unread {
            0 => "No unread mail".to_owned(),
            1 => "1 unread message".to_owned(),
            n => format!("{n} unread messages"),
        };
        ToolTip { title: "Postie".into(), description, ..Default::default() }
    }

    fn activate(&mut self, _x: i32, _y: i32) {
        self.hub.show(None);
    }

    fn menu(&self) -> Vec<MenuItem<Self>> {
        vec![
            StandardItem {
                label: "Open Postie".into(),
                activate: Box::new(|tray: &mut Self| tray.hub.show(None)),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "New Message".into(),
                activate: Box::new(|tray: &mut Self| {
                    tray.hub.show(Some(Event::Compose {
                        to: String::new(),
                        subject: String::new(),
                        body: String::new(),
                    }))
                }),
                ..Default::default()
            }
            .into(),
            StandardItem {
                label: "Check for New Mail".into(),
                activate: Box::new(|tray: &mut Self| tray.hub.sync_now()),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            CheckmarkItem {
                label: "Do Not Disturb".into(),
                checked: self.dnd,
                activate: Box::new(|tray: &mut Self| {
                    tray.dnd = !tray.dnd;
                    tray.hub.set_do_not_disturb(tray.dnd);
                }),
                ..Default::default()
            }
            .into(),
            MenuItem::Separator,
            StandardItem {
                label: "Quit Postie".into(),
                activate: Box::new(|tray: &mut Self| tray.hub.quit()),
                ..Default::default()
            }
            .into(),
        ]
    }
}
