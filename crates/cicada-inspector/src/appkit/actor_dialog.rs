//! Native actor action controls, confirmations and refusal alerts.

use std::ptr::NonNull;

use block2::RcBlock;
use objc2::rc::{Retained, Weak};
use objc2::runtime::AnyObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAccessibility, NSAlert, NSAlertFirstButtonReturn, NSAlertSecondButtonReturn, NSEvent, NSEventMask,
    NSEventModifierFlags, NSModalResponse, NSWindow,
};
use objc2_foundation::{NSInteger, NSString, ns_string};

use super::Target;
use crate::actors::PaneView;
use crate::actors::actions::{Action, Alert, Confirmation};

const ACTIONS: [Action; 5] = [Action::Pause, Action::Resume, Action::CancelTurn, Action::Purge, Action::Delete];
const RETURN: u16 = 36;
const ESCAPE: u16 = 53;
const KEYPAD_ENTER: u16 = 76;

pub(super) struct Controls<'a> {
    pub selected: bool,
    pub busy: bool,
    pub notice: Option<&'a str>,
}

pub(super) fn action(tag: NSInteger) -> Option<Action> {
    ACTIONS.get(usize::try_from(tag).ok()?).copied()
}

pub(super) fn buttons(pane: &PaneView) -> Vec<(String, NSInteger)> {
    Action::offered(pane)
        .map(|action| {
            let tag = ACTIONS.iter().position(|&value| value == action).expect("the buttons cover every actor action");
            let ellipsis = if action.asks_first() { "…" } else { "" };
            (format!("{}{ellipsis}", action.title()), tag as NSInteger)
        })
        .collect()
}

pub(super) struct Dialog {
    alert: Retained<NSAlert>,
    monitor: Option<Retained<AnyObject>>,
}

impl Dialog {
    pub fn confirm(target: &Target, confirmation: Confirmation) {
        let Confirmation { id, action, text } = confirmation;
        let alert = NSAlert::new(target.mtm());
        alert.setMessageText(&NSString::from_str(action.title()));
        alert.setInformativeText(&NSString::from_str(&text));
        let cancel = alert.addButtonWithTitle(ns_string!("Cancel"));
        cancel.setAccessibilityLabel(Some(ns_string!("Cancel")));
        cancel.setKeyEquivalent(ns_string!("\r"));
        cancel.setKeyEquivalentModifierMask(NSEventModifierFlags::empty());
        let title = NSString::from_str(action.title());
        let destructive = alert.addButtonWithTitle(&title);
        destructive.setAccessibilityLabel(Some(&title));
        destructive.setKeyEquivalent(ns_string!(""));
        destructive.setHasDestructiveAction(true);
        destructive.setRefusesFirstResponder(true);

        let monitor = cancel_keys(&alert.window());
        Self::open(target, alert, monitor, Some(id));
    }

    pub fn refusal(target: &Target, refusal: &Alert) {
        let alert = NSAlert::new(target.mtm());
        alert.setMessageText(&NSString::from_str(&refusal.title));
        if let Some(detail) = &refusal.detail {
            alert.setInformativeText(&NSString::from_str(detail));
        }
        alert.addButtonWithTitle(ns_string!("OK"));
        Self::open(target, alert, None, None);
    }

    fn open(target: &Target, alert: Retained<NSAlert>, monitor: Option<Retained<AnyObject>>, id: Option<u64>) {
        debug_assert!(target.ivars().actor_dialog.borrow().is_none());
        let parent = &target.window().window;
        let weak = Weak::new(target);
        let sheet_alert = Weak::new(&*alert);
        let completion = RcBlock::new(move |response: NSModalResponse| {
            if let (Some(target), Some(alert)) = (weak.load(), sheet_alert.load()) {
                target.finish_actor_dialog(&alert, id, response == NSAlertSecondButtonReturn);
            }
        });
        target.ivars().actor_dialog.replace(Some(Self { alert: alert.clone(), monitor }));
        alert.beginSheetModalForWindow_completionHandler(parent, Some(&completion));
    }

    pub fn dismiss(self, parent: &NSWindow) {
        let window = self.alert.window();
        if parent.attachedSheet().is_some_and(|attached| std::ptr::eq(&*attached, &*window)) {
            parent.endSheet_returnCode(&window, NSAlertFirstButtonReturn);
        }
    }

    pub fn matches(&self, alert: &NSAlert) -> bool {
        std::ptr::eq(&*self.alert, alert)
    }
}

impl Drop for Dialog {
    fn drop(&mut self) {
        if let Some(monitor) = self.monitor.take() {
            // SAFETY: This is the token of our local monitor, removed once on the sheet's main thread.
            unsafe { NSEvent::removeMonitor(&monitor) };
        }
    }
}

/// AppKit gives the first button Return. Explicitly send Escape (and keypad Enter) to the same button.
fn cancel_keys(window: &NSWindow) -> Option<Retained<AnyObject>> {
    let sheet = Weak::new(window);
    let handler = RcBlock::new(move |event: NonNull<NSEvent>| {
        let Some(mtm) = MainThreadMarker::new() else { return event.as_ptr() };
        let Some(sheet) = sheet.load() else { return event.as_ptr() };
        // SAFETY: AppKit supplies a live event for the duration of this call.
        let key = unsafe { event.as_ref() };
        if matches!(key.keyCode(), RETURN | ESCAPE | KEYPAD_ENTER)
            && key.window(mtm).is_some_and(|window| std::ptr::eq(&*window, &*sheet))
            && let Some(parent) = sheet.sheetParent()
        {
            parent.endSheet_returnCode(&sheet, NSAlertFirstButtonReturn);
            std::ptr::null_mut()
        } else {
            event.as_ptr()
        }
    });
    // SAFETY: The block returns the event AppKit gave it or null to consume the key.
    unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(NSEventMask::KeyDown, &handler) }
}
