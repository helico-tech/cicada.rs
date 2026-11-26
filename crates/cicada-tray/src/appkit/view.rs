//! Rendering of the native status item and menu.

use std::cell::{Cell, RefCell};

use objc2::rc::Retained;
use objc2::runtime::{ProtocolObject, Sel};
use objc2::{MainThreadMarker, MainThreadOnly, sel};
use objc2_app_kit::{
    NSAccessibility, NSColor, NSControlStateValue, NSControlStateValueOff, NSControlStateValueOn, NSFont, NSImage,
    NSLineBreakMode, NSMenu, NSMenuItem, NSMenuItemBadge, NSStatusBar, NSStatusBarButton, NSStatusItem, NSSwitch,
    NSTextField, NSVariableStatusItemLength, NSView,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString, ns_string};

use super::Target;
use crate::menu::{Choice, Header, MenuModel, RecentRow, RunningRow, running_row_text};

pub(super) struct MenuView {
    pub root: Retained<NSMenu>,
    button: Retained<NSStatusBarButton>,
    header: HeaderView,
    running_section: Retained<NSMenuItem>,
    context_section: Retained<NSMenuItem>,
    registered: Retained<NSMenu>,
    recent: Retained<NSMenu>,
    recent_item: Retained<NSMenuItem>,
    quit: Retained<NSMenuItem>,
    subtitles: bool,
    running_rows: RefCell<Rows<RunningRow>>,
    context_rows: RefCell<Rows<Choice<String>>>,
    registered_rows: RefCell<Rows<Choice<&'static str>>>,
    recent_rows: RefCell<Rows<RecentRow>>,
    contexts_enabled: Cell<bool>,
    failures: Cell<usize>,
}

impl MenuView {
    pub fn new(
        mtm: MainThreadMarker,
        bar: &NSStatusBar,
        title: &str,
        subtitles: bool,
    ) -> Option<(Retained<NSStatusItem>, Self)> {
        let status = bar.statusItemWithLength(NSVariableStatusItemLength);
        let Some(button) = status.button(mtm) else {
            bar.removeStatusItem(&status);
            return None;
        };
        let title = NSString::from_str(title);
        if let Some(image) =
            NSImage::imageWithSystemSymbolName_accessibilityDescription(ns_string!("bolt.fill"), Some(&title))
        {
            image.setTemplate(true);
            button.setImage(Some(&image));
        } else {
            button.setTitle(&title);
        }
        button.setAccessibilityLabel(Some(&title));

        let root = native_menu(mtm, &title);
        let header = HeaderView::new(mtm);
        let running_section = NSMenuItem::sectionHeaderWithTitle(ns_string!("Running"), mtm);
        let context_section = NSMenuItem::sectionHeaderWithTitle(ns_string!("Context"), mtm);
        let (registered, registered_item) = submenu(mtm, ns_string!("Registered Types"));
        let (recent, recent_item) = submenu(mtm, ns_string!("Recent Turns"));
        let settings = item(mtm, ns_string!("Settings…"), None, ns_string!(","));
        settings.setEnabled(false);
        let quit = item(mtm, ns_string!("Quit"), Some(sel!(trayQuit:)), ns_string!("q"));
        for row in [
            &header.item,
            &NSMenuItem::separatorItem(mtm),
            &running_section,
            &NSMenuItem::separatorItem(mtm),
            &context_section,
            &NSMenuItem::separatorItem(mtm),
            &registered_item,
            &recent_item,
            &NSMenuItem::separatorItem(mtm),
            &settings,
            &quit,
        ] {
            root.addItem(row);
        }
        status.setMenu(Some(&root));
        let view = Self {
            root,
            button,
            header,
            running_section,
            context_section,
            registered,
            recent,
            recent_item,
            quit,
            // objc2 exposes these selectors without version attributes. The SDK declares section headers and badges
            // in 14.0, and subtitles in 14.4. The caller already checked for 14.0.
            subtitles,
            running_rows: RefCell::default(),
            context_rows: RefCell::default(),
            registered_rows: RefCell::default(),
            recent_rows: RefCell::default(),
            contexts_enabled: Cell::new(true),
            failures: Cell::new(0),
        };
        Some((status, view))
    }

    pub fn bind(&self, target: &Target) {
        self.root.setDelegate(Some(ProtocolObject::from_ref(target)));
        // SAFETY: The target outlives menu tracking and implements both selectors with the sender type wired here.
        unsafe {
            self.header.switch.setTarget(Some(target));
            self.header.switch.setAction(Some(sel!(trayToggle:)));
            self.header.item.setTarget(Some(target));
            self.quit.setTarget(Some(target));
        }
    }

    pub fn render(&self, target: &Target, model: MenuModel) {
        let mtm = target.mtm();
        self.header.update(model.header);
        if self.button.appearsDisabled() != model.dimmed {
            self.button.setAppearsDisabled(model.dimmed);
        }

        let start = self.root.indexOfItem(&self.running_section) + 1;
        self.running_rows.borrow_mut().sync(
            &self.root,
            start,
            model.running,
            |old, new| old.turn == new.turn,
            |_| item(mtm, ns_string!(""), None, ns_string!("")),
            |item, row| {
                let (title, subtitle) = running_row_text(row, self.subtitles);
                item.setTitle(&NSString::from_str(&title));
                if let Some(subtitle) = subtitle {
                    item.setSubtitle(Some(&NSString::from_str(&subtitle)));
                }
            },
        );

        let start = self.root.indexOfItem(&self.context_section) + 1;
        let mut rows = self.context_rows.borrow_mut();
        rows.sync(
            &self.root,
            start,
            model.contexts,
            |old, new| old.name == new.name,
            |row| {
                let item = item(mtm, &NSString::from_str(&row.name), Some(sel!(trayPickContext:)), ns_string!(""));
                // SAFETY: The target implements trayPickContext: with an NSMenuItem sender and owns this menu.
                unsafe { item.setTarget(Some(target)) };
                item.setEnabled(model.contexts_enabled);
                item
            },
            |item, row| item.setState(mark(row.marked)),
        );
        if self.contexts_enabled.replace(model.contexts_enabled) != model.contexts_enabled {
            for (_, item) in &rows.items {
                item.setEnabled(model.contexts_enabled);
            }
            self.quit.setEnabled(model.contexts_enabled);
        }
        drop(rows);

        self.registered_rows.borrow_mut().sync(
            &self.registered,
            0,
            model.registered_types,
            |old, new| old.name == new.name,
            |row| item(mtm, &NSString::from_str(row.name), None, ns_string!("")),
            |item, row| item.setState(mark(row.marked)),
        );
        self.recent_rows.borrow_mut().sync(
            &self.recent,
            0,
            model.recent,
            |old, new| old.turn == new.turn,
            |row| {
                let title = format!("{} · {}", row.turn.turn.actor, row.outcome);
                item(mtm, &NSString::from_str(&title), None, ns_string!(""))
            },
            |_, _| {},
        );
        if self.failures.replace(model.failures) != model.failures {
            let badge = (model.failures > 0).then(|| NSMenuItemBadge::alertsWithCount(model.failures as isize));
            self.recent_item.setBadge(badge.as_deref());
        }
    }
}

fn native_menu(mtm: MainThreadMarker, title: &NSString) -> Retained<NSMenu> {
    let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), title);
    menu.setAutoenablesItems(false);
    menu
}

fn item(mtm: MainThreadMarker, title: &NSString, action: Option<Sel>, key: &NSString) -> Retained<NSMenuItem> {
    // SAFETY: Every action passed here is a selector defined by Target and paired with that target before tracking.
    unsafe { NSMenuItem::initWithTitle_action_keyEquivalent(NSMenuItem::alloc(mtm), title, action, key) }
}

fn submenu(mtm: MainThreadMarker, title: &NSString) -> (Retained<NSMenu>, Retained<NSMenuItem>) {
    let menu = native_menu(mtm, title);
    let item = item(mtm, title, None, ns_string!(""));
    item.setSubmenu(Some(&menu));
    (menu, item)
}

fn mark(marked: bool) -> NSControlStateValue {
    if marked { NSControlStateValueOn } else { NSControlStateValueOff }
}

struct HeaderView {
    item: Retained<NSMenuItem>,
    title: Retained<NSTextField>,
    subtitle: Retained<NSTextField>,
    switch: Retained<NSSwitch>,
    shown: RefCell<Option<Header>>,
}

impl HeaderView {
    fn new(mtm: MainThreadMarker) -> Self {
        let view = NSView::initWithFrame(NSView::alloc(mtm), rect(0.0, 0.0, 280.0, 54.0));
        let title = NSTextField::labelWithString(ns_string!(""), mtm);
        title.setFrame(rect(14.0, 28.0, 202.0, 18.0));
        title.setFont(Some(&NSFont::boldSystemFontOfSize(13.0)));
        title.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
        let subtitle = NSTextField::labelWithString(ns_string!(""), mtm);
        subtitle.setFrame(rect(14.0, 9.0, 202.0, 17.0));
        subtitle.setTextColor(Some(&NSColor::secondaryLabelColor()));
        subtitle.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
        let switch = NSSwitch::initWithFrame(NSSwitch::alloc(mtm), rect(224.0, 16.0, 44.0, 24.0));
        switch.setAccessibilityLabel(Some(ns_string!("Worker connection")));
        view.addSubview(&title);
        view.addSubview(&subtitle);
        view.addSubview(&switch);
        // AppKit gives the custom view mouse events and handles keyboard selection on the item itself.
        let item = item(mtm, ns_string!("Worker Connection"), Some(sel!(trayToggle:)), ns_string!(""));
        item.setView(Some(&view));
        Self { item, title, subtitle, switch, shown: RefCell::default() }
    }

    fn update(&self, header: Header) {
        let mut shown = self.shown.borrow_mut();
        if shown.as_ref() != Some(&header) {
            self.title.setStringValue(&NSString::from_str(&header.title));
            self.subtitle.setStringValue(&NSString::from_str(&header.subtitle));
            let tooltip = NSString::from_str(header.error.as_deref().unwrap_or(&header.subtitle));
            self.subtitle.setToolTip(Some(&tooltip));
            self.switch.setEnabled(header.enabled);
            self.item.setEnabled(header.enabled);
        }
        let state = mark(header.on);
        if self.switch.state() != state {
            self.switch.setState(state);
        }
        *shown = Some(header);
    }
}

fn rect(x: f64, y: f64, width: f64, height: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, y), NSSize::new(width, height))
}

struct Rows<T> {
    items: Vec<(T, Retained<NSMenuItem>)>,
}

impl<T> Default for Rows<T> {
    fn default() -> Self {
        Self { items: Vec::new() }
    }
}

impl<T: Eq> Rows<T> {
    fn sync(
        &mut self,
        menu: &NSMenu,
        start: isize,
        new: Vec<T>,
        same: impl Fn(&T, &T) -> bool,
        make: impl Fn(&T) -> Retained<NSMenuItem>,
        update: impl Fn(&NSMenuItem, &T),
    ) {
        let mut old = std::mem::take(&mut self.items);
        // Remove missing rows first so shifts do not move or replace surviving rows in AppKit.
        old.retain(|(row, item)| {
            let keep = new.iter().any(|next| same(row, next));
            if !keep {
                menu.removeItem(item);
            }
            keep
        });
        for (offset, row) in new.into_iter().enumerate() {
            let index = start + offset as isize;
            let item = match old.iter().position(|(previous, _)| same(previous, &row)) {
                Some(position) => {
                    let (previous, item) = old.remove(position);
                    if position != 0 {
                        menu.removeItem(&item);
                        menu.insertItem_atIndex(&item, index);
                    }
                    if previous != row {
                        update(&item, &row);
                    }
                    item
                }
                None => {
                    let item = make(&row);
                    update(&item, &row);
                    menu.insertItem_atIndex(&item, index);
                    item
                }
            };
            self.items.push((row, item));
        }
        for (_, item) in old {
            menu.removeItem(&item);
        }
    }
}
