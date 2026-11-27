//! The Settings window: a native form over [`SettingsForm`], in the grouped style of System Settings.

use std::cell::RefCell;

use objc2::rc::Retained;
use objc2::runtime::Sel;
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAccessibility, NSApplication, NSBackingStoreType, NSBox, NSBoxType, NSButton, NSColor, NSControlStateValueOff,
    NSControlStateValueOn, NSEvent, NSEventModifierFlags, NSEventType, NSFont, NSLineBreakMode, NSMenu, NSMenuItem,
    NSPopUpButton, NSResponder, NSSecureTextField, NSStepper, NSSwitch, NSTextField, NSTitlePosition, NSView, NSWindow,
    NSWindowStyleMask,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString, ns_string};

use super::Target;
use crate::settings::{ApplyError, Field, MAX_CONCURRENCY, SettingsForm};

const WIDTH: f64 = 520.0;
const MARGIN: f64 = 20.0;
const ROW: f64 = 32.0;
const NOTE: f64 = 18.0;
const LABEL: f64 = 140.0;
const BOX_WIDTH: f64 = WIDTH - 2.0 * MARGIN;
const CONTROL_X: f64 = 16.0 + LABEL;
const CONTROL_WIDTH: f64 = BOX_WIDTH - CONTROL_X - 16.0;
/// The tag of the pop-up's Add… item; contexts have their index.
const ADD_TAG: isize = -1;
/// The key code of Esc.
const ESCAPE: u16 = 53;

define_class!(
    // SAFETY: NSWindow has no subclassing requirements beyond forwarding to super, and the class has no ivars or Drop.
    #[unsafe(super(NSWindow, NSResponder, objc2_foundation::NSObject))]
    #[name = "CicadaTraySettingsWindow"]
    #[thread_kind = MainThreadOnly]
    struct SettingsPanel;

    impl SettingsPanel {
        /// Esc and ⌘W close the window, also while a field is being edited; everything else goes to NSWindow.
        // SAFETY: The signature is NSWindow's sendEvent:.
        #[unsafe(method(sendEvent:))]
        fn send_event(&self, event: &NSEvent) {
            if event.r#type() == NSEventType::KeyDown {
                let flags = event.modifierFlags() & NSEventModifierFlags::DeviceIndependentFlagsMask;
                let modified = NSEventModifierFlags::Command | NSEventModifierFlags::Control | NSEventModifierFlags::Option;
                let key = event.charactersIgnoringModifiers().map(|key| key.to_string()).unwrap_or_default();
                let escape = event.keyCode() == ESCAPE && !flags.intersects(modified);
                let close = flags & modified == NSEventModifierFlags::Command && key.eq_ignore_ascii_case("w");
                if escape || close {
                    self.close();
                    return;
                }
            }
            // SAFETY: Forwards the same event to NSWindow's implementation.
            unsafe { msg_send![super(self), sendEvent: event] }
        }
    }
);

/// The window, its controls and the form they show; made at the first Settings….
pub(super) struct SettingsWindow {
    window: Retained<SettingsPanel>,
    contexts: Retained<NSPopUpButton>,
    context_error: Retained<NSTextField>,
    server: Retained<NSTextField>,
    server_note: Retained<NSTextField>,
    token: Retained<NSSecureTextField>,
    switches: Vec<(&'static str, Retained<NSSwitch>)>,
    types_error: Retained<NSTextField>,
    concurrency: Retained<NSTextField>,
    stepper: Retained<NSStepper>,
    concurrency_error: Retained<NSTextField>,
    status: Retained<NSTextField>,
    apply: Retained<NSButton>,
    sheet: Retained<NSWindow>,
    sheet_name: Retained<NSTextField>,
    sheet_error: Retained<NSTextField>,
    form: RefCell<Option<SettingsForm>>,
}

impl SettingsWindow {
    pub fn new(target: &Target, title: &str, registered: &[&'static str]) -> Self {
        let mtm = target.mtm();
        let worker_height = 8.0 + registered.len() as f64 * ROW + NOTE + ROW + NOTE + 8.0;
        let connection_height = 8.0 + ROW + NOTE + ROW + NOTE + ROW + 8.0;
        let height =
            MARGIN + 22.0 + connection_height + 16.0 + 22.0 + worker_height + 12.0 + 34.0 + 12.0 + ROW + MARGIN;
        let window = panel(mtm, &format!("{title} Settings"), height);
        let content = window.contentView().expect("a window has a content view");
        let mut top = MARGIN;

        content.addSubview(&section(mtm, "Connection", height, top));
        top += 22.0;
        let (connection, inner) = group(mtm, height, top, connection_height);
        content.addSubview(&connection);
        let mut row = 8.0;
        inner.addSubview(&row_label(mtm, "Context", connection_height, row));
        let contexts = NSPopUpButton::initWithFrame_pullsDown(
            NSPopUpButton::alloc(mtm),
            frame(CONTROL_X, connection_height, row + 3.0, CONTROL_WIDTH, 26.0),
            false,
        );
        contexts.setAccessibilityLabel(Some(ns_string!("Context")));
        action(&contexts, target, sel!(settingsContext:));
        inner.addSubview(&contexts);
        row += ROW;
        let context_error = note(mtm, connection_height, row, true);
        inner.addSubview(&context_error);
        row += NOTE;
        inner.addSubview(&row_label(mtm, "Server", connection_height, row));
        let server = NSTextField::textFieldWithString(ns_string!(""), mtm);
        server.setFrame(frame(CONTROL_X, connection_height, row + 5.0, CONTROL_WIDTH, 22.0));
        server.setPlaceholderString(Some(ns_string!("https://actors.example.com")));
        server.setAccessibilityLabel(Some(ns_string!("Server")));
        inner.addSubview(&server);
        row += ROW;
        let server_note = note(mtm, connection_height, row, false);
        inner.addSubview(&server_note);
        row += NOTE;
        inner.addSubview(&row_label(mtm, "Token", connection_height, row));
        let token = NSSecureTextField::initWithFrame(
            NSSecureTextField::alloc(mtm),
            frame(CONTROL_X, connection_height, row + 5.0, CONTROL_WIDTH, 22.0),
        );
        token.setPlaceholderString(Some(ns_string!("None")));
        token.setAccessibilityLabel(Some(ns_string!("Token")));
        inner.addSubview(&token);
        top += connection_height + 16.0;

        content.addSubview(&section(mtm, "Worker", height, top));
        top += 22.0;
        let (worker, inner) = group(mtm, height, top, worker_height);
        content.addSubview(&worker);
        let mut row = 8.0;
        let switches = registered
            .iter()
            .map(|&name| {
                inner.addSubview(&row_label(mtm, name, worker_height, row));
                let switch = NSSwitch::initWithFrame(
                    NSSwitch::alloc(mtm),
                    frame(BOX_WIDTH - 16.0 - 44.0, worker_height, row + 4.0, 44.0, 24.0),
                );
                switch.setAccessibilityLabel(Some(&NSString::from_str(&format!("Run {name}"))));
                inner.addSubview(&switch);
                row += ROW;
                (name, switch)
            })
            .collect();
        let types_error = note(mtm, worker_height, row, true);
        inner.addSubview(&types_error);
        row += NOTE;
        inner.addSubview(&row_label(mtm, "Turns at once", worker_height, row));
        let concurrency = NSTextField::textFieldWithString(ns_string!(""), mtm);
        concurrency.setFrame(frame(BOX_WIDTH - 16.0 - 22.0 - 64.0, worker_height, row + 5.0, 60.0, 22.0));
        concurrency.setAccessibilityLabel(Some(ns_string!("Turns at once")));
        action(&concurrency, target, sel!(settingsConcurrency:));
        inner.addSubview(&concurrency);
        let stepper = NSStepper::initWithFrame(
            NSStepper::alloc(mtm),
            frame(BOX_WIDTH - 16.0 - 19.0, worker_height, row + 3.0, 19.0, 27.0),
        );
        stepper.setMinValue(1.0);
        stepper.setMaxValue(MAX_CONCURRENCY as f64);
        stepper.setIncrement(1.0);
        stepper.setAccessibilityLabel(Some(ns_string!("Turns at once")));
        action(&stepper, target, sel!(settingsStep:));
        inner.addSubview(&stepper);
        row += ROW;
        let concurrency_error = note(mtm, worker_height, row, true);
        inner.addSubview(&concurrency_error);
        top += worker_height + 12.0;

        let status = NSTextField::wrappingLabelWithString(ns_string!(""), mtm);
        status.setFrame(frame(MARGIN, height, top, BOX_WIDTH, 34.0));
        status.setFont(Some(&NSFont::systemFontOfSize(11.0)));
        status.setTextColor(Some(&NSColor::secondaryLabelColor()));
        content.addSubview(&status);
        top += 34.0 + 12.0;
        let apply = button(mtm, target, "Apply and Reconnect", sel!(settingsApply:));
        apply.setKeyEquivalent(ns_string!("\r"));
        apply.setFrame(frame(WIDTH - MARGIN - 180.0, height, top, 180.0, ROW));
        content.addSubview(&apply);
        let revert = button(mtm, target, "Revert", sel!(settingsRevert:));
        revert.setFrame(frame(WIDTH - MARGIN - 180.0 - 8.0 - 100.0, height, top, 100.0, ROW));
        content.addSubview(&revert);
        window.setInitialFirstResponder(Some(&server));

        let (sheet, sheet_name, sheet_error) = add_sheet(mtm, target);
        Self {
            window,
            contexts,
            context_error,
            server,
            server_note,
            token,
            switches,
            types_error,
            concurrency,
            stepper,
            concurrency_error,
            status,
            apply,
            sheet,
            sheet_name,
            sheet_error,
            form: RefCell::default(),
        }
    }

    /// Brings the window to the front and makes it key; a closed window first reads the files again.
    pub fn show(&self, form: impl FnOnce() -> Result<SettingsForm, String>) {
        if !self.window.isVisible() {
            self.replace(form());
            self.window.center();
        }
        NSApplication::sharedApplication(self.window.mtm()).activate();
        self.window.makeKeyAndOrderFront(None);
    }

    fn replace(&self, form: Result<SettingsForm, String>) {
        self.clear_errors();
        match form {
            Ok(form) => {
                self.status.setStringValue(ns_string!(""));
                self.form.replace(Some(form));
            }
            Err(error) => {
                self.status.setStringValue(&NSString::from_str(&error));
                self.form.replace(None);
            }
        }
        self.fill();
    }

    /// Shows the values of the form.
    fn fill(&self) {
        let form = self.form.borrow();
        self.apply.setEnabled(form.is_some());
        let mtm = self.window.mtm();
        self.contexts.removeAllItems();
        let menu = self.contexts.menu().expect("a pop-up button has a menu");
        let (names, selected) = form.as_ref().map(|f| (f.contexts(), f.selected())).unwrap_or_default();
        for (index, name) in names.iter().enumerate() {
            let item = menu_item(mtm, name);
            item.setTag(index as isize);
            menu.addItem(&item);
            if Some(name.as_str()) == selected {
                self.contexts.selectItem(Some(&item));
            }
        }
        if !names.is_empty() {
            menu.addItem(&NSMenuItem::separatorItem(mtm));
        }
        let add = menu_item(mtm, "Add…");
        add.setTag(ADD_TAG);
        menu.addItem(&add);
        if selected.is_none() {
            self.contexts.selectItem(None);
        }
        let values = form.as_ref().map(|f| &f.edited);
        self.server.setStringValue(&NSString::from_str(values.map_or("", |v| &v.server)));
        self.token.setStringValue(&NSString::from_str(values.map_or("", |v| &v.token)));
        for (name, switch) in &self.switches {
            let on = values.is_some_and(|v| v.types.iter().any(|t| t == name));
            switch.setState(if on { NSControlStateValueOn } else { NSControlStateValueOff });
        }
        let concurrency = values.map_or("", |v| &v.concurrency);
        self.concurrency.setStringValue(&NSString::from_str(concurrency));
        if let Ok(n) = concurrency.parse::<f64>() {
            self.stepper.setDoubleValue(n);
        }
        let note = form.as_ref().and_then(SettingsForm::environment_note).unwrap_or_default();
        set_note(&self.server_note, &note, false);
    }

    /// Copies the controls into the form's edited values.
    fn read(&self, form: &mut SettingsForm) {
        form.edited.server = self.server.stringValue().to_string();
        form.edited.token = self.token.stringValue().to_string();
        form.edited.types = self
            .switches
            .iter()
            .filter(|(_, switch)| switch.state() == NSControlStateValueOn)
            .map(|(name, _)| name.to_string())
            .collect();
        form.edited.concurrency = self.concurrency.stringValue().to_string();
    }

    fn clear_errors(&self) {
        for label in [&self.context_error, &self.types_error, &self.concurrency_error] {
            set_note(label, "", true);
        }
        let note = self.form.borrow().as_ref().and_then(SettingsForm::environment_note).unwrap_or_default();
        set_note(&self.server_note, &note, false);
    }

    fn show_errors(&self, errors: &[(Field, String)]) {
        for (field, error) in errors {
            let label = match field {
                Field::Context => &self.context_error,
                Field::Server => &self.server_note,
                Field::Types => &self.types_error,
                Field::Concurrency => &self.concurrency_error,
            };
            set_note(label, error, true);
        }
    }

    /// A pick in the context pop-up: Add… opens the sheet, a context shows its values.
    pub fn picked_context(&self) {
        let tag = self.contexts.selectedItem().map_or(ADD_TAG, |item| item.tag());
        if tag == ADD_TAG {
            self.sheet_name.setStringValue(ns_string!(""));
            set_note(&self.sheet_error, "", true);
            self.window.beginSheet_completionHandler(&self.sheet, None);
            return;
        }
        let name = self.form.borrow().as_ref().and_then(|form| form.contexts().get(tag as usize).cloned());
        let Some(name) = name else { return };
        let result = self.form.borrow_mut().as_mut().map(|form| form.select(&name));
        if let Some(Err(error)) = result {
            self.status.setStringValue(&NSString::from_str(&error));
        }
        self.clear_errors();
        self.fill();
    }

    /// Add in the sheet: a valid name becomes the selected new context; an invalid one is named in the sheet.
    pub fn add(&self) {
        let name = self.sheet_name.stringValue().to_string();
        let result = self.form.borrow_mut().as_mut().map(|form| form.add(name.trim()));
        match result {
            Some(Err(error)) => set_note(&self.sheet_error, &error, true),
            _ => {
                self.window.endSheet(&self.sheet);
                self.clear_errors();
                self.fill();
            }
        }
    }

    /// Cancel in the sheet: the pop-up shows the selected context again.
    pub fn cancel_add(&self) {
        self.window.endSheet(&self.sheet);
        self.fill();
    }

    pub fn stepped(&self) {
        self.concurrency.setStringValue(&NSString::from_str(&self.stepper.integerValue().to_string()));
    }

    pub fn typed_concurrency(&self) {
        if let Ok(n) = self.concurrency.stringValue().to_string().trim().parse::<f64>() {
            self.stepper.setDoubleValue(n);
        }
    }

    /// Revert: the values on disk.
    pub fn revert(&self, form: impl FnOnce() -> Result<SettingsForm, String>) {
        let result = self.form.borrow_mut().as_mut().map(SettingsForm::revert);
        match result {
            Some(Ok(())) => {
                self.status.setStringValue(ns_string!(""));
                self.clear_errors();
                self.fill();
            }
            Some(Err(error)) => self.replace(Err(error)),
            None => self.replace(form()),
        }
    }

    /// Apply: the context to reconnect to when both files are saved; otherwise the errors are shown and the window
    /// keeps the values.
    pub fn apply(&self) -> Option<String> {
        let result = {
            let mut form = self.form.borrow_mut();
            let form = form.as_mut()?;
            self.read(form);
            form.apply().map(|name| {
                let reread = form.select(&name);
                (name, reread)
            })
        };
        self.clear_errors();
        match result {
            Ok((name, reread)) => {
                let text = if reread.is_ok() { "Saved." } else { "Saved; the files cannot be read again." };
                self.status.setStringValue(&NSString::from_str(text));
                Some(name)
            }
            Err(ApplyError::Invalid(errors)) => {
                self.status.setStringValue(ns_string!(""));
                self.show_errors(&errors);
                None
            }
            Err(ApplyError::Failed(error) | ApplyError::Partial(error)) => {
                self.status.setStringValue(&NSString::from_str(&error));
                None
            }
        }
    }

    /// Shows the form's values after a successful apply.
    pub fn refill(&self) {
        self.clear_errors();
        self.fill();
    }

    pub fn status(&self, text: &str) {
        self.status.setStringValue(&NSString::from_str(text));
    }
}

fn panel(mtm: MainThreadMarker, title: &str, height: f64) -> Retained<SettingsPanel> {
    let style = NSWindowStyleMask::Titled | NSWindowStyleMask::Closable;
    let rect = NSRect::new(NSPoint::ZERO, NSSize::new(WIDTH, height));
    // SAFETY: NSWindow's designated initializer, with a valid style mask and backing store.
    let window: Retained<SettingsPanel> = unsafe {
        msg_send![
            SettingsPanel::alloc(mtm),
            initWithContentRect: rect,
            styleMask: style,
            backing: NSBackingStoreType::Buffered,
            defer: false
        ]
    };
    // SAFETY: The window is kept in a Retained for the life of the app, so closing must not release it.
    unsafe { window.setReleasedWhenClosed(false) };
    window.setTitle(&NSString::from_str(title));
    window
}

/// The Add… sheet: its window, the name field and the error label.
fn add_sheet(
    mtm: MainThreadMarker,
    target: &Target,
) -> (Retained<NSWindow>, Retained<NSTextField>, Retained<NSTextField>) {
    let (width, height) = (360.0, 150.0);
    let rect = NSRect::new(NSPoint::ZERO, NSSize::new(width, height));
    // SAFETY: NSWindow's designated initializer, with a valid style mask and backing store.
    let sheet = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            rect,
            NSWindowStyleMask::Titled,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    // SAFETY: The sheet is kept in a Retained for the life of the app, so ending it must not release it.
    unsafe { sheet.setReleasedWhenClosed(false) };
    let content = sheet.contentView().expect("a window has a content view");
    let label = NSTextField::labelWithString(ns_string!("Name of the new context"), mtm);
    label.setFrame(frame(MARGIN, height, MARGIN, width - 2.0 * MARGIN, 18.0));
    content.addSubview(&label);
    let name = NSTextField::textFieldWithString(ns_string!(""), mtm);
    name.setFrame(frame(MARGIN, height, MARGIN + 24.0, width - 2.0 * MARGIN, 22.0));
    name.setPlaceholderString(Some(ns_string!("staging")));
    name.setAccessibilityLabel(Some(ns_string!("Name of the new context")));
    content.addSubview(&name);
    let error = note(mtm, height, MARGIN + 50.0, true);
    error.setFrame(frame(MARGIN, height, MARGIN + 50.0, width - 2.0 * MARGIN, NOTE));
    content.addSubview(&error);
    let add = button(mtm, target, "Add", sel!(settingsAddOk:));
    add.setKeyEquivalent(ns_string!("\r"));
    add.setFrame(frame(width - MARGIN - 90.0, height, height - MARGIN - ROW, 90.0, ROW));
    content.addSubview(&add);
    let cancel = button(mtm, target, "Cancel", sel!(settingsAddCancel:));
    cancel.setKeyEquivalent(ns_string!("\u{1b}"));
    cancel.setFrame(frame(width - MARGIN - 90.0 - 8.0 - 90.0, height, height - MARGIN - ROW, 90.0, ROW));
    content.addSubview(&cancel);
    sheet.setInitialFirstResponder(Some(&name));
    (sheet, name, error)
}

/// A frame `top` points below the top of a parent `parent` points high, in AppKit's bottom-up coordinates.
fn frame(x: f64, parent: f64, top: f64, width: f64, height: f64) -> NSRect {
    NSRect::new(NSPoint::new(x, parent - top - height), NSSize::new(width, height))
}

fn section(mtm: MainThreadMarker, title: &str, height: f64, top: f64) -> Retained<NSTextField> {
    let label = NSTextField::labelWithString(&NSString::from_str(title), mtm);
    label.setFont(Some(&NSFont::boldSystemFontOfSize(13.0)));
    label.setFrame(frame(MARGIN + 4.0, height, top, BOX_WIDTH, 18.0));
    label
}

/// A rounded group box and its content view.
fn group(mtm: MainThreadMarker, height: f64, top: f64, group_height: f64) -> (Retained<NSBox>, Retained<NSView>) {
    let group = NSBox::initWithFrame(NSBox::alloc(mtm), frame(MARGIN, height, top, BOX_WIDTH, group_height));
    group.setBoxType(NSBoxType::Custom);
    group.setTitlePosition(NSTitlePosition::NoTitle);
    group.setCornerRadius(10.0);
    group.setBorderWidth(1.0);
    group.setBorderColor(&NSColor::separatorColor());
    group.setFillColor(&NSColor::quaternarySystemFillColor());
    group.setContentViewMargins(NSSize::ZERO);
    let inner = group.contentView().expect("a box has a content view");
    (group, inner)
}

fn row_label(mtm: MainThreadMarker, title: &str, parent: f64, top: f64) -> Retained<NSTextField> {
    let label = NSTextField::labelWithString(&NSString::from_str(title), mtm);
    label.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    label.setFrame(frame(16.0, parent, top + 7.0, LABEL - 8.0, 18.0));
    label
}

/// A small label under a control, for an error (red) or a note (secondary).
fn note(mtm: MainThreadMarker, parent: f64, top: f64, error: bool) -> Retained<NSTextField> {
    let label = NSTextField::labelWithString(ns_string!(""), mtm);
    label.setFont(Some(&NSFont::systemFontOfSize(11.0)));
    label.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    label.setFrame(frame(CONTROL_X, parent, top, CONTROL_WIDTH, NOTE - 2.0));
    set_note(&label, "", error);
    label
}

fn set_note(label: &NSTextField, text: &str, error: bool) {
    let color = if error { NSColor::systemRedColor() } else { NSColor::secondaryLabelColor() };
    label.setTextColor(Some(&color));
    label.setStringValue(&NSString::from_str(text));
    label.setToolTip((!text.is_empty()).then(|| NSString::from_str(text)).as_deref());
}

fn button(mtm: MainThreadMarker, target: &Target, title: &str, selector: Sel) -> Retained<NSButton> {
    // SAFETY: Target implements the selector with an NSButton sender and outlives the window.
    unsafe { NSButton::buttonWithTitle_target_action(&NSString::from_str(title), Some(target), Some(selector), mtm) }
}

fn menu_item(mtm: MainThreadMarker, title: &str) -> Retained<NSMenuItem> {
    // SAFETY: The item has no action; the pop-up button sends its own.
    unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str(title),
            None,
            ns_string!(""),
        )
    }
}

/// Wires `control` to `selector` of `target`.
fn action(control: &objc2_app_kit::NSControl, target: &Target, selector: Sel) {
    // SAFETY: Target implements the selector with the control as the sender and outlives the window.
    unsafe {
        control.setTarget(Some(target));
        control.setAction(Some(selector));
    }
}

/// The application's main menu: invisible for an accessory app, but it gives text fields the standard edit
/// shortcuts (⌘X, ⌘C, ⌘V, ⌘A, ⌘Z), which go to the first responder.
pub(super) fn edit_menu(mtm: MainThreadMarker) -> Retained<NSMenu> {
    let main = NSMenu::initWithTitle(NSMenu::alloc(mtm), ns_string!(""));
    let edit = NSMenu::initWithTitle(NSMenu::alloc(mtm), ns_string!("Edit"));
    for (title, selector, key) in [
        ("Undo", sel!(undo:), "z"),
        ("Redo", sel!(redo:), "Z"),
        ("Cut", sel!(cut:), "x"),
        ("Copy", sel!(copy:), "c"),
        ("Paste", sel!(paste:), "v"),
        ("Select All", sel!(selectAll:), "a"),
    ] {
        // SAFETY: These are standard responder actions sent to the first responder, which ignores ones it lacks.
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &NSString::from_str(title),
                Some(selector),
                &NSString::from_str(key),
            )
        };
        edit.addItem(&item);
    }
    for (title, submenu) in [("App", NSMenu::initWithTitle(NSMenu::alloc(mtm), ns_string!(""))), ("Edit", edit)] {
        let item = menu_item(mtm, title);
        item.setSubmenu(Some(&submenu));
        main.addItem(&item);
    }
    main
}
