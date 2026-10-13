//! A feature with no actor and nothing to ask one - the chrome hears about
//! the world through the bus and keeps a count.

use guinea::prelude::*;

use crate::events::ProcessKilled;

use super::contracts::{self, Chrome};

feature! {
    pub TabsFeature {
        exports { contracts::Tabs }
    }
}

#[installs]
fn tabs(cx: &Install<str>) -> anyhow::Result<TabsFeature> {
    let chrome = cx.state::<contracts::Tabs>().plain();

    let count = cx
        .scope
        .peek::<contracts::Tabs>()
        .map_or(0, |tabs| tabs.borrow().install_count);
    chrome.push(Chrome::Installed(count + 1));
    chrome.push(Chrome::Reached(cx.params().to_string()));

    let killed = chrome.clone();
    cx.subscribe::<ProcessKilled>(move |ev: ProcessKilled| {
        killed.push(Chrome::Killed(ev.name));
    });

    Ok(TabsFeature(chrome))
}
