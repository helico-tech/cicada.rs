//! A read-only text view of highlighted JSON with a notice when a state, payload or schema is cut.

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSAccessibility, NSAutoresizingMaskOptions, NSColor, NSFont, NSFontAttributeName, NSForegroundColorAttributeName,
    NSScrollView, NSTextField, NSTextView, NSView,
};
use objc2_foundation::{NSPoint, NSRange, NSRect, NSSize, NSString, ns_string};

use super::content::{Style, color};
use crate::json::{CUT, Json, Token, utf16_ranges};

/// The view, its notice, and the colours of the tokens, made once.
pub(super) struct JsonText {
    /// The view to place: the notice over the text.
    pub root: Retained<NSView>,
    notice: Retained<NSTextField>,
    scroll: Retained<NSScrollView>,
    text: Retained<NSTextView>,
    colors: [Retained<NSColor>; 5],
    /// What the JSON is, for the notice: `state`, `payload` or `schema`.
    what: &'static str,
}

impl JsonText {
    /// A view `size` large whose text VoiceOver names `label`.
    pub(super) fn new(mtm: MainThreadMarker, size: NSSize, label: &str, what: &'static str) -> Self {
        let root = NSView::initWithFrame(NSView::alloc(mtm), NSRect::new(NSPoint::ZERO, size));
        let scroll = NSTextView::scrollableTextView(mtm);
        scroll.setFrame(NSRect::new(NSPoint::ZERO, size));
        scroll.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
        );
        let text = scroll
            .documentView()
            .and_then(|view| view.downcast::<NSTextView>().ok())
            .expect("a scrollable text view holds a text view");
        text.setEditable(false);
        text.setSelectable(true);
        text.setRichText(false);
        text.setUsesFontPanel(false);
        text.setTextContainerInset(NSSize::new(6.0, 8.0));
        text.setFont(Some(&mono()));
        text.setAccessibilityLabel(Some(&NSString::from_str(label)));
        root.addSubview(&scroll);
        let notice = NSTextField::wrappingLabelWithString(ns_string!(""), mtm);
        notice.setFont(Some(&NSFont::systemFontOfSize(11.0)));
        notice.setTextColor(Some(&NSColor::secondaryLabelColor()));
        notice.setHidden(true);
        notice.setAutoresizingMask(
            NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewMinYMargin,
        );
        root.addSubview(&notice);
        let colors = [
            NSColor::controlAccentColor(),
            NSColor::systemGreenColor(),
            NSColor::systemOrangeColor(),
            NSColor::systemPurpleColor(),
            NSColor::secondaryLabelColor(),
        ];
        Self { root, notice, scroll, text, colors, what }
    }

    /// A plain text instead of JSON, secondary unless `style` says otherwise.
    pub(super) fn show_plain(&self, text: &str, style: Option<Style>) {
        self.notice.setHidden(true);
        self.layout(0.0);
        self.text.setString(&NSString::from_str(text));
        self.text.setTextColor(Some(&color(style.unwrap_or(Style::Secondary))));
    }

    /// The JSON with its runs applied as UTF-16 ranges, in one batch of the text storage.
    pub(super) fn show(&self, json: &Json) {
        let what = self.what;
        let notice = json
            .cut
            .then(|| format!("The {what} is longer than {} KiB; this shows its first {} KiB.", CUT / 1024, CUT / 1024));
        self.notice.setHidden(notice.is_none());
        self.notice.setStringValue(&NSString::from_str(notice.as_deref().unwrap_or("")));
        self.layout(if notice.is_some() { 34.0 } else { 0.0 });
        // SAFETY: A text view made by `scrollableTextView` has a text storage.
        let Some(storage) = (unsafe { self.text.textStorage() }) else { return };
        let ranges = utf16_ranges(&json.text, &json.runs);
        let font = mono();
        let label = NSColor::labelColor();
        storage.beginEditing();
        storage.replaceCharactersInRange_withString(NSRange::new(0, storage.length()), &NSString::from_str(&json.text));
        let all = NSRange::new(0, storage.length());
        // SAFETY: The attribute names are AppKit's constants, the values a font and colours, and every range lies
        // within the text just set: the runs are on character boundaries of that text.
        unsafe {
            storage.addAttribute_value_range(NSFontAttributeName, as_object(&*font), all);
            storage.addAttribute_value_range(NSForegroundColorAttributeName, as_object(&*label), all);
            for (location, length, token) in ranges {
                let color = &self.colors[token_index(token)];
                storage.addAttribute_value_range(
                    NSForegroundColorAttributeName,
                    as_object(&**color),
                    NSRange::new(location, length),
                );
            }
        }
        storage.endEditing();
    }

    /// Places the text under a notice `notice` high.
    fn layout(&self, notice: f64) {
        let size = self.root.frame().size;
        let notice_frame = NSRect::new(
            NSPoint::new(8.0, size.height - notice + 4.0),
            NSSize::new(size.width - 16.0, (notice - 8.0).max(0.0)),
        );
        self.notice.setFrame(notice_frame);
        self.scroll.setFrame(NSRect::new(NSPoint::ZERO, NSSize::new(size.width, size.height - notice)));
    }
}

fn mono() -> Retained<NSFont> {
    NSFont::monospacedSystemFontOfSize_weight(12.0, 0.0)
}

fn token_index(token: Token) -> usize {
    match token {
        Token::Key => 0,
        Token::String => 1,
        Token::Number => 2,
        Token::Literal => 3,
        Token::Punctuation => 4,
    }
}

fn as_object<T: AsRef<AnyObject>>(value: &T) -> &AnyObject {
    value.as_ref()
}
