//! The Send Message sheet: the message type pop-up, the chosen schema read only beside a plain-text JSON editor, the
//! local verdict, the server's refusal, and Cancel and Send. It has no default button: Return never closes it, ⌘Return
//! sends and Escape cancels, also from the editor (the target binds the editor's `cancelOperation:` to Cancel).

use std::cell::{Cell, RefCell};
use std::sync::Arc;

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{MainThreadOnly, msg_send, sel};
use objc2_app_kit::{
    NSAccessibility, NSAlert, NSBackingStoreType, NSButton, NSControl, NSEventModifierFlags, NSFont, NSLineBreakMode,
    NSPopUpButton, NSScrollView, NSTextField, NSTextView, NSView, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{NSInteger, NSPoint, NSRect, NSSize, NSString, ns_string};

use super::Target;
use super::content::{FlippedView, Style, color, label};
use super::json_text::JsonText;
use crate::actors::actions::{Choices, START, SendView, Verdict};
use crate::json::Json;

const WIDTH: f64 = 760.0;
const HEIGHT: f64 = 560.0;
const MARGIN: f64 = 20.0;
const COLUMNS_TOP: f64 = 92.0;
const COLUMNS_HEIGHT: f64 = 330.0;

/// The open sheet and the views it updates.
pub(super) struct SendSheet {
    /// The model's sheet id.
    pub id: u64,
    window: Retained<NSWindow>,
    chooser: Retained<NSPopUpButton>,
    note: Retained<NSTextField>,
    retry: Retained<NSButton>,
    schema: JsonText,
    schema_note: Retained<NSTextField>,
    editor: Retained<NSTextView>,
    verdict: Retained<NSTextField>,
    refusal: Retained<NSTextField>,
    cancel: Retained<NSButton>,
    send: Retained<NSButton>,
    choices_shown: RefCell<Option<Choices>>,
    schema_shown: RefCell<Option<Arc<Json>>>,
    alert_shown: Cell<u64>,
    alert: RefCell<Option<Retained<NSAlert>>>,
}

impl SendSheet {
    /// Builds the sheet for the model's sheet `id` and shows it on `parent`, with the editor focused.
    pub fn open(target: &Target, parent: &NSWindow, id: u64, address: &str) -> Self {
        let mtm = target.mtm();
        let rect = NSRect::new(NSPoint::ZERO, NSSize::new(WIDTH, HEIGHT));
        // SAFETY: NSWindow's designated initializer, with a style mask and backing AppKit accepts for a sheet.
        let window: Retained<NSWindow> = unsafe {
            msg_send![
                NSWindow::alloc(mtm),
                initWithContentRect: rect,
                styleMask: NSWindowStyleMask::Titled,
                backing: NSBackingStoreType::Buffered,
                defer: false
            ]
        };
        // SAFETY: The sheet is held in a Retained, so closing must not release it.
        unsafe { window.setReleasedWhenClosed(false) };
        window.setTitle(ns_string!("Send Message"));
        // SAFETY: NSView's designated initializer; the class adds no ivars.
        let content: Retained<FlippedView> = unsafe { msg_send![FlippedView::alloc(mtm), initWithFrame: rect] };
        window.setContentView(Some(&content));

        let inner = WIDTH - 2.0 * MARGIN;
        let title = label(mtm, &format!("Send Message to {address}"), NSFont::boldSystemFontOfSize(15.0), Style::Plain);
        title.setLineBreakMode(NSLineBreakMode::ByTruncatingMiddle);
        place(&content, &title, MARGIN, MARGIN, inner, 22.0);

        let caption = label(mtm, "Message type", NSFont::systemFontOfSize(13.0), Style::Plain);
        place(&content, &caption, MARGIN, 54.0, 100.0, 22.0);
        let chooser = NSPopUpButton::initWithFrame_pullsDown(NSPopUpButton::alloc(mtm), rect, false);
        chooser.setAccessibilityLabel(Some(ns_string!("Message type")));
        place(&content, &chooser, MARGIN + 104.0, 52.0, 260.0, 26.0);
        let note = label(mtm, "", NSFont::systemFontOfSize(13.0), Style::Secondary);
        place(&content, &note, MARGIN + 104.0, 54.0, inner - 104.0 - 100.0, 22.0);
        // SAFETY: Retry sends inspectorSendRetry: to the target, which implements it with the button as sender and
        // clears the button's target when the sheet closes.
        let retry = unsafe {
            NSButton::buttonWithTitle_target_action(
                ns_string!("Retry"),
                Some(target),
                Some(sel!(inspectorSendRetry:)),
                mtm,
            )
        };
        place(&content, &retry, WIDTH - MARGIN - 90.0, 52.0, 90.0, 26.0);
        // SAFETY: The pop-up sends inspectorSendType: to the target, which implements it with the pop-up as sender and
        // clears the pop-up's target when the sheet closes.
        unsafe {
            NSControl::setTarget(&chooser, Some(target));
            NSControl::setAction(&chooser, Some(sel!(inspectorSendType:)));
        }

        let column = (inner - 16.0) / 2.0;
        let schema = JsonText::new(mtm, NSSize::new(column, COLUMNS_HEIGHT), "JSON Schema", "schema");
        place(&content, &schema.root, MARGIN, COLUMNS_TOP, column, COLUMNS_HEIGHT);
        let schema_note = label(mtm, "", NSFont::systemFontOfSize(12.0), Style::Secondary);
        place(&content, &schema_note, MARGIN, COLUMNS_TOP + COLUMNS_HEIGHT + 6.0, column, 18.0);

        let scroll = NSTextView::scrollableTextView(mtm);
        let editor = scroll
            .documentView()
            .and_then(|view| view.downcast::<NSTextView>().ok())
            .expect("a scrollable text view holds a text view");
        editor.setRichText(false);
        editor.setUsesFontPanel(false);
        editor.setAllowsUndo(true);
        editor.setFont(Some(&NSFont::monospacedSystemFontOfSize_weight(12.0, 0.0)));
        editor.setTextContainerInset(NSSize::new(6.0, 8.0));
        // Typed JSON stays as typed: no curly quotes, long dashes, replacements or corrections.
        editor.setAutomaticQuoteSubstitutionEnabled(false);
        editor.setAutomaticDashSubstitutionEnabled(false);
        editor.setAutomaticTextReplacementEnabled(false);
        editor.setAutomaticSpellingCorrectionEnabled(false);
        editor.setAccessibilityLabel(Some(ns_string!("Payload")));
        editor.setString(&NSString::from_str(START));
        editor.setDelegate(Some(ProtocolObject::from_ref(target)));
        place_scroll(&content, &scroll, MARGIN + column + 16.0, column);
        let verdict = label(mtm, "", NSFont::systemFontOfSize(12.0), Style::Secondary);
        place(&content, &verdict, MARGIN + column + 16.0, COLUMNS_TOP + COLUMNS_HEIGHT + 6.0, column, 18.0);

        let refusal = NSTextField::wrappingLabelWithString(ns_string!(""), mtm);
        refusal.setFont(Some(&NSFont::systemFontOfSize(12.0)));
        refusal.setTextColor(Some(&color(Style::Error)));
        refusal.setSelectable(true);
        place(&content, &refusal, MARGIN, COLUMNS_TOP + COLUMNS_HEIGHT + 30.0, inner, 54.0);

        // SAFETY: Cancel sends inspectorSendCancel: to the target, as Retry does.
        let cancel = unsafe {
            NSButton::buttonWithTitle_target_action(
                ns_string!("Cancel"),
                Some(target),
                Some(sel!(inspectorSendCancel:)),
                mtm,
            )
        };
        cancel.setKeyEquivalent(ns_string!("\u{1b}"));
        cancel.setKeyEquivalentModifierMask(NSEventModifierFlags::empty());
        // SAFETY: Send sends inspectorSendMessage: to the target, as Retry does.
        let send = unsafe {
            NSButton::buttonWithTitle_target_action(
                ns_string!("Send"),
                Some(target),
                Some(sel!(inspectorSendMessage:)),
                mtm,
            )
        };
        // ⌘Return, not Return: Return in the editor is a new line, and the sheet has no default button.
        send.setKeyEquivalent(ns_string!("\r"));
        send.setKeyEquivalentModifierMask(NSEventModifierFlags::Command);
        send.setToolTip(Some(ns_string!("Send (⌘Return)")));
        send.setEnabled(false);
        place(&content, &send, WIDTH - MARGIN - 90.0, HEIGHT - MARGIN - 28.0, 90.0, 28.0);
        place(&content, &cancel, WIDTH - MARGIN - 188.0, HEIGHT - MARGIN - 28.0, 90.0, 28.0);

        let sheet = Self {
            id,
            window,
            chooser,
            note,
            retry,
            schema,
            schema_note,
            editor,
            verdict,
            refusal,
            cancel,
            send,
            choices_shown: RefCell::default(),
            schema_shown: RefCell::default(),
            alert_shown: Cell::new(0),
            alert: RefCell::default(),
        };
        parent.beginSheet_completionHandler(&sheet.window, None);
        sheet.window.makeFirstResponder(Some(&sheet.editor));
        sheet
    }

    /// True for this sheet's payload editor.
    pub fn is_editor(&self, view: &NSTextView) -> bool {
        std::ptr::eq(&*self.editor, view)
    }

    /// Fills the sheet from `view`; the editor keeps what the operator typed.
    pub fn show(&self, view: &SendView) {
        let ready = matches!(view.choices, Choices::Ready(_));
        let changed = self.choices_shown.borrow().as_ref() != Some(&view.choices);
        if changed {
            self.chooser.removeAllItems();
            if let Choices::Ready(names) = &view.choices {
                for name in names {
                    self.chooser.addItemWithTitle(&NSString::from_str(name));
                }
            }
            self.choices_shown.replace(Some(view.choices.clone()));
        }
        if ready && self.chooser.indexOfSelectedItem() != view.chosen as NSInteger {
            self.chooser.selectItemAtIndex(view.chosen as NSInteger);
        }
        self.chooser.setHidden(!ready);
        // The draft and the type stay as sent while a send is out.
        self.chooser.setEnabled(!view.sending);
        self.editor.setEditable(!view.sending);
        let note = view.choices.note();
        self.note.setHidden(note.is_none());
        self.note.setStringValue(&NSString::from_str(note.as_deref().unwrap_or("")));
        self.note.setTextColor(Some(&color(if matches!(view.choices, Choices::Loading) {
            Style::Secondary
        } else {
            Style::Error
        })));
        self.retry.setHidden(!matches!(view.choices, Choices::Failed(_)));

        let shown = self.schema_shown.borrow().clone();
        match (&view.schema, &shown) {
            (Some(json), Some(before)) if Arc::ptr_eq(json, before) => {}
            (Some(json), _) => self.schema.show(json),
            (None, _) => self.schema.show_plain(if ready { "Compiling…" } else { "" }, None),
        }
        self.schema_shown.replace(view.schema.clone());
        self.schema_note.setStringValue(&NSString::from_str(view.schema_note().unwrap_or("")));

        let (text, style) = match (&view.verdict, view.sending) {
            _ if !ready => (String::new(), Style::Secondary),
            (_, true) => ("Sending…".into(), Style::Secondary),
            (Verdict::Checking, _) => ("Checking…".into(), Style::Secondary),
            (Verdict::Passes, _) => ("The payload matches the schema.".into(), Style::Secondary),
            (Verdict::Unchecked, _) => ("The payload is not checked against the schema.".into(), Style::Secondary),
            (Verdict::Refused(why), _) => (format!("The payload {why}."), Style::Error),
        };
        self.verdict.setStringValue(&NSString::from_str(&text));
        self.verdict.setTextColor(Some(&color(style)));
        let refusal = view.refusal.as_ref().map(|alert| match &alert.detail {
            Some(detail) => format!("{}: {detail}", alert.title),
            None => alert.title.clone(),
        });
        self.refusal.setStringValue(&NSString::from_str(refusal.as_deref().unwrap_or("")));
        // No Send where nothing can be sent: no message types, an unknown type, or types not read.
        self.send.setHidden(!ready);
        self.send.setEnabled(view.can_send());

        if let Some(alert) = &view.alert
            && view.sends != self.alert_shown.get()
        {
            self.alert_shown.set(view.sends);
            let panel = NSAlert::new(self.window.mtm());
            panel.setMessageText(&NSString::from_str(&alert.title));
            if let Some(detail) = &alert.detail {
                panel.setInformativeText(&NSString::from_str(detail));
            }
            panel.addButtonWithTitle(ns_string!("OK"));
            self.alert.replace(Some(panel.clone()));
            panel.beginSheetModalForWindow_completionHandler(&self.window, None);
        }
    }

    /// Ends the sheet on `parent`, with any alert over it, and detaches the target.
    pub fn dismiss(self, parent: &NSWindow) {
        if let Some(alert) = self.alert.take() {
            let window = alert.window();
            if self.window.attachedSheet().is_some_and(|attached| std::ptr::eq(&*attached, &*window)) {
                self.window.endSheet(&window);
            }
        }
        self.editor.setDelegate(None);
        // SAFETY: The controls accept a nil target, and the target is still alive.
        unsafe {
            NSControl::setTarget(&self.chooser, None);
            NSControl::setTarget(&self.retry, None);
            NSControl::setTarget(&self.cancel, None);
            NSControl::setTarget(&self.send, None);
        }
        if parent.attachedSheet().is_some_and(|attached| std::ptr::eq(&*attached, &*self.window)) {
            parent.endSheet(&self.window);
        }
        self.window.orderOut(None);
    }
}

fn place(content: &FlippedView, view: &NSView, x: f64, y: f64, width: f64, height: f64) {
    view.setFrame(NSRect::new(NSPoint::new(x, y), NSSize::new(width, height)));
    content.addSubview(view);
}

fn place_scroll(content: &FlippedView, scroll: &NSScrollView, x: f64, width: f64) {
    place(content, scroll, x, COLUMNS_TOP, width, COLUMNS_HEIGHT);
}
