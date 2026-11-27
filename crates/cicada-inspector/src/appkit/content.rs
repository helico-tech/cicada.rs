//! The content pane: the Overview, laid out from the top in a flipped view, or "Coming soon"; and the layout that
//! the Actors page's pane uses too.

use cicada::api::NodeView;
use objc2::rc::Retained;
use objc2::{MainThreadMarker, MainThreadOnly, define_class};
use objc2_app_kit::{
    NSAccessibility, NSAutoresizingMaskOptions, NSBox, NSBoxType, NSColor, NSFont, NSLineBreakMode, NSResponder,
    NSScrollView, NSTextAlignment, NSTextField, NSTitlePosition, NSView,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use crate::model::{Count, Overview, Part, Section, TURNS_PAGE, Tile, View, age, count_text, tiles};

define_class!(
    // SAFETY: NSView has no subclassing requirements, and the class has no ivars or Drop.
    #[unsafe(super(NSView, NSResponder, objc2_foundation::NSObject))]
    #[name = "CicadaInspectorFlippedView"]
    #[thread_kind = MainThreadOnly]
    pub(super) struct FlippedView;

    impl FlippedView {
        /// Content is laid out from the top.
        // SAFETY: The signature is NSView's isFlipped.
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }
    }
);

const MARGIN: f64 = 24.0;
const LINE: f64 = 20.0;
const TILE_HEIGHT: f64 = 96.0;
const GAP: f64 = 12.0;
const COUNT_WIDTH: f64 = 120.0;

/// Draws `section` of `view` into `document`, as wide as `scroll` shows.
pub(super) fn render(document: &FlippedView, scroll: &NSScrollView, section: Section, view: &View) {
    for subview in document.subviews().to_vec() {
        subview.removeFromSuperview();
    }
    let size = scroll.contentSize();
    let mut layout = Layout { document, mtm: document.mtm(), y: MARGIN, width: size.width.max(480.0) };
    if section == Section::Overview {
        overview(&mut layout, view);
    } else if !section.ready() {
        layout.y = (size.height / 2.0 - 40.0).max(MARGIN);
        layout.centered("Coming soon", 20.0, true);
        layout.centered(&format!("{} comes in a later version of the inspector.", section.title()), 13.0, false);
    }
    document.setFrame(NSRect::new(NSPoint::ZERO, NSSize::new(layout.width, (layout.y + MARGIN).max(size.height))));
}

fn overview(layout: &mut Layout<'_>, view: &View) {
    if let Some(error) = &view.contexts_error {
        layout.text(error, Style::Error);
    }
    if !view.overrides.is_empty() {
        layout.text(&format!("{}.", super::override_text(&view.overrides)), Style::Secondary);
    }
    if let Some(error) = &view.target_error {
        layout.text(error, Style::Error);
        return;
    }
    let Some(overview) = &view.overview else {
        layout.text(if view.refreshing { "Loading…" } else { "Not refreshed yet." }, Style::Secondary);
        return;
    };
    if view.refreshing {
        layout.text("Refreshing…", Style::Secondary);
    }
    layout.y += 4.0;
    layout.tiles(&tiles(&overview.stats, chrono::Utc::now()));

    layout.heading("Health");
    match &overview.health {
        Part::Ready(true) => layout.text("Healthy: the node and its store answer.", Style::Plain),
        Part::Ready(false) => layout.text("Unhealthy: the store does not answer.", Style::Error),
        Part::Failed(error) => layout.text(error, Style::Error),
    }

    nodes(layout, overview);
    turns(layout, overview);

    match &overview.types {
        Part::Ready(types) => {
            layout.heading(&format!("Types ({})", types.len()));
            if types.is_empty() {
                layout.text("No types are registered.", Style::Secondary);
            }
            for row in types {
                let tip = match &row.actors {
                    Count::Failed(error) => Some(error.as_str()),
                    Count::NotCounted => Some("Not counted: past the 50 types counted, or past the 10 s refresh."),
                    Count::Counted(_) => None,
                };
                layout.row(&row.name, &count_text(&row.actors), tip);
            }
        }
        Part::Failed(error) => {
            layout.heading("Types");
            layout.text(error, Style::Error);
        }
    }
}

fn nodes(layout: &mut Layout<'_>, overview: &Overview) {
    match &overview.nodes {
        Part::Ready(nodes) => {
            layout.heading(&format!("Nodes ({})", nodes.len()));
            if nodes.is_empty() {
                layout.text("No nodes.", Style::Secondary);
            }
            for node in nodes {
                layout.row(&node_text(node), &format!("{} running", node.running_turns), None);
            }
        }
        Part::Failed(error) => {
            layout.heading("Nodes");
            layout.text(error, Style::Error);
        }
    }
}

fn node_text(node: &NodeView) -> String {
    let status = format!("{:?}", node.status).to_lowercase();
    let hosted = if node.hosted.is_empty() { "sends only".to_owned() } else { node.hosted.join(", ") };
    let host = node.hostname.as_deref().map(|h| format!(" on {h}")).unwrap_or_default();
    format!("{}{host} · {status} · {hosted}", node.id)
}

fn turns(layout: &mut Layout<'_>, overview: &Overview) {
    match &overview.turns {
        Part::Ready(turns) => {
            let count = if turns.more { format!("first {TURNS_PAGE}") } else { turns.items.len().to_string() };
            layout.heading(&format!("Running turns ({count})"));
            if turns.items.is_empty() {
                layout.text("No turns are running.", Style::Secondary);
            }
            let now = chrono::Utc::now();
            for turn in &turns.items {
                let cancel = if turn.cancel_requested { " · cancel requested" } else { "" };
                let left = format!("{} · {} · on {}{cancel}", turn.actor, turn.message_id, turn.node);
                layout.row(&left, &format!("lease {}", age(turn.lease_until, now)), None);
            }
            if turns.more {
                layout.text(&format!("More turns run; this shows the first {TURNS_PAGE}."), Style::Secondary);
            }
        }
        Part::Failed(error) => {
            layout.heading("Running turns");
            layout.text(error, Style::Error);
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Style {
    Plain,
    Secondary,
    Error,
}

/// Places views from the top of the document, one under the other.
pub(super) struct Layout<'a> {
    document: &'a FlippedView,
    mtm: MainThreadMarker,
    y: f64,
    width: f64,
}

impl<'a> Layout<'a> {
    /// Empties `document` for a layout `width` wide, from `margin` below its top.
    pub(super) fn new(document: &'a FlippedView, width: f64, margin: f64) -> Self {
        for subview in document.subviews().to_vec() {
            subview.removeFromSuperview();
        }
        Self { document, mtm: document.mtm(), y: margin, width }
    }

    /// Sizes the document to what was placed, at least `height` high; returns the height.
    pub(super) fn finish(self, height: f64) -> f64 {
        let height = (self.y + GAP).max(height);
        self.document.setFrame(NSRect::new(NSPoint::ZERO, NSSize::new(self.width, height)));
        height
    }

    fn inner(&self) -> f64 {
        self.width - 2.0 * MARGIN
    }

    fn add(&self, view: &NSView, x: f64, width: f64, height: f64) {
        view.setFrame(NSRect::new(NSPoint::new(x, self.y), NSSize::new(width, height)));
        self.document.addSubview(view);
    }

    pub(super) fn heading(&mut self, text: &str) {
        self.y += 18.0;
        let label = label(self.mtm, text, NSFont::boldSystemFontOfSize(15.0), Style::Plain);
        self.add(&label, MARGIN, self.inner(), LINE + 2.0);
        self.y += LINE + 6.0;
    }

    pub(super) fn text(&mut self, text: &str, style: Style) {
        let label = NSTextField::wrappingLabelWithString(&NSString::from_str(text), self.mtm);
        label.setFont(Some(&NSFont::systemFontOfSize(13.0)));
        label.setTextColor(Some(&color(style)));
        label.setSelectable(true);
        label.setPreferredMaxLayoutWidth(self.inner());
        let height = label.fittingSize().height.max(LINE);
        self.add(&label, MARGIN, self.inner(), height);
        self.y += height + 4.0;
    }

    fn centered(&mut self, text: &str, size: f64, bold: bool) {
        let font = if bold { NSFont::boldSystemFontOfSize(size) } else { NSFont::systemFontOfSize(size) };
        let label = label(self.mtm, text, font, if bold { Style::Plain } else { Style::Secondary });
        label.setAlignment(NSTextAlignment::Center);
        self.add(&label, MARGIN, self.inner(), size + 10.0);
        self.y += size + 14.0;
    }

    /// A line with a text on the left and a count on the right; `tip` explains the count.
    pub(super) fn row(&mut self, left: &str, right: &str, tip: Option<&str>) {
        let name = label(self.mtm, left, NSFont::systemFontOfSize(13.0), Style::Plain);
        name.setLineBreakMode(NSLineBreakMode::ByTruncatingMiddle);
        name.setToolTip(Some(&NSString::from_str(left)));
        self.add(&name, MARGIN, self.inner() - COUNT_WIDTH - GAP, LINE);
        let style = if tip.is_some() { Style::Secondary } else { Style::Plain };
        let count = label(self.mtm, right, NSFont::monospacedDigitSystemFontOfSize_weight(13.0, 0.0), style);
        count.setAlignment(NSTextAlignment::Right);
        if let Some(tip) = tip {
            count.setToolTip(Some(&NSString::from_str(tip)));
        }
        self.add(&count, MARGIN + self.inner() - COUNT_WIDTH, COUNT_WIDTH, LINE);
        self.y += LINE + 2.0;
    }

    /// The four tiles side by side.
    fn tiles(&mut self, tiles: &[Tile; 4]) {
        let width = (self.inner() - 3.0 * GAP) / 4.0;
        for (index, tile) in tiles.iter().enumerate() {
            let x = MARGIN + index as f64 * (width + GAP);
            let frame = NSBox::new(self.mtm);
            frame.setBoxType(NSBoxType::Custom);
            frame.setTitlePosition(NSTitlePosition::NoTitle);
            frame.setCornerRadius(10.0);
            frame.setBorderColor(&NSColor::separatorColor());
            frame.setFillColor(&NSColor::controlBackgroundColor());
            self.add(&frame, x, width, TILE_HEIGHT);
            let inner = frame.contentView().expect("a box has a content view");
            let place = |view: &NSView, top: f64, height: f64| {
                view.setFrame(NSRect::new(
                    NSPoint::new(12.0, TILE_HEIGHT - top - height),
                    NSSize::new(width - 24.0, height),
                ));
                view.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
                inner.addSubview(view);
            };
            place(&label(self.mtm, tile.label, NSFont::systemFontOfSize(12.0), Style::Secondary), 10.0, 16.0);
            let value =
                label(self.mtm, &tile.value, NSFont::monospacedDigitSystemFontOfSize_weight(26.0, 0.3), Style::Plain);
            place(&value, 28.0, 32.0);
            let note = label(
                self.mtm,
                &tile.note,
                NSFont::systemFontOfSize(11.0),
                if tile.error { Style::Error } else { Style::Secondary },
            );
            note.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
            note.setToolTip(Some(&NSString::from_str(&tile.note)));
            place(&note, 64.0, 16.0);
            frame.setAccessibilityLabel(Some(&NSString::from_str(&format!(
                "{}: {}, {}",
                tile.label, tile.value, tile.note
            ))));
        }
        self.y += TILE_HEIGHT + 4.0;
    }
}

pub(super) fn label(
    mtm: MainThreadMarker,
    text: &str,
    font: objc2::rc::Retained<NSFont>,
    style: Style,
) -> Retained<NSTextField> {
    let label = NSTextField::labelWithString(&NSString::from_str(text), mtm);
    label.setFont(Some(&font));
    label.setTextColor(Some(&color(style)));
    label.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    label
}

pub(super) fn color(style: Style) -> Retained<NSColor> {
    match style {
        Style::Plain => NSColor::labelColor(),
        Style::Secondary => NSColor::secondaryLabelColor(),
        Style::Error => NSColor::systemRedColor(),
    }
}
