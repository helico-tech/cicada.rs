//! Rendering of the native status item and menu, as the boards in `openspec/changes/tray-menu-mockup/mockup/` show
//! them: native items with an image, a title, a subtitle and a badge for the text on the right.

use std::cell::{Cell, RefCell};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject, Sel};
use objc2::{AnyThread, MainThreadMarker, MainThreadOnly, sel};
use objc2_app_kit::{
    NSAccessibility, NSAttributedStringNSStringDrawing, NSAutoresizingMaskOptions, NSColor, NSControlSize,
    NSControlStateValue, NSControlStateValueOff, NSControlStateValueOn, NSFont, NSFontAttributeName,
    NSFontWeightSemibold, NSForegroundColorAttributeName, NSImage, NSLineBreakMode, NSMenu, NSMenuItem,
    NSMenuItemBadge, NSProgressIndicator, NSProgressIndicatorStyle, NSStatusBar, NSStatusBarButton, NSStatusItem,
    NSSwitch, NSTextAlignment, NSTextField, NSVariableStatusItemLength, NSView,
};
use objc2_foundation::{NSAttributedString, NSDictionary, NSPoint, NSRect, NSSize, NSString, ns_string};

use super::Target;
use super::icons::{self, Icons};
use crate::menu::{
    ACTOR_TYPES, BACKGROUND_TASKS, Bar, CHOOSE_TYPES, ContextRow, Header, InspectorItem, MenuModel, RECENT, RecentRow,
    Row, RunningRow, STOP_NOW_HELP, TypeRow, row_text,
};

/// The boards' menu width.
const WIDTH: f64 = 300.0;
/// The boards' horizontal padding of a row's text.
const PADDING: f64 = 14.0;

pub(super) struct MenuView {
    pub root: Retained<NSMenu>,
    button: Retained<NSStatusBarButton>,
    header: HeaderView,
    failure: Retained<NSMenuItem>,
    failure_separator: Retained<NSMenuItem>,
    running_section: Retained<NSMenuItem>,
    running_separator: Retained<NSMenuItem>,
    context_section: Retained<NSMenuItem>,
    context_separator: Retained<NSMenuItem>,
    registered_item: Retained<NSMenuItem>,
    registered: Retained<NSMenu>,
    recent_item: Retained<NSMenuItem>,
    recent: Retained<NSMenu>,
    submenus_separator: Retained<NSMenuItem>,
    connect: Retained<NSMenuItem>,
    stop_now: Retained<NSMenuItem>,
    stop_now_help: Retained<NSMenuItem>,
    stop_now_separator: Retained<NSMenuItem>,
    choose_types: Retained<NSMenuItem>,
    inspector: Option<Retained<NSMenuItem>>,
    settings: Retained<NSMenuItem>,
    quit: Retained<NSMenuItem>,
    subtitles: bool,
    running_rows: RefCell<Rows<RunningRow>>,
    context_rows: RefCell<Rows<ContextRow>>,
    registered_rows: RefCell<Rows<TypeRow>>,
    recent_rows: RefCell<Rows<RecentRow>>,
    shown: RefCell<Shown>,
    icons: Icons,
    /// The status item's images: not connected, connected.
    status_images: [Retained<NSImage>; 2],
    dark: Cell<bool>,
}

/// What the fixed items show now, so a render changes AppKit only where the model changed.
#[derive(Default, PartialEq)]
struct Shown {
    connected: bool,
    failure: Option<Row>,
    running: Option<(&'static str, String)>,
    contexts_enabled: bool,
    registered_right: Option<String>,
    recent_right: Option<String>,
    connect: Option<String>,
    quit: (String, bool),
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
        let status_images = [false, true].map(|connected| icons::status_image(connected, &title));
        button.setImage(Some(&status_images[0]));
        button.setAccessibilityLabel(Some(&title));

        let root = native_menu(mtm, &title);
        root.setMinimumWidth(WIDTH);
        let header = HeaderView::new(mtm);
        let failure = item(mtm, ns_string!(""), None, ns_string!(""));
        // A disabled item styled as the boards' headers, as a badge on a native section header may not draw.
        let running_section = item(mtm, ns_string!("Running"), None, ns_string!(""));
        running_section.setEnabled(false);
        let context_section = NSMenuItem::sectionHeaderWithTitle(ns_string!("Context"), mtm);
        let (registered, registered_item) = submenu(mtm, ns_string!("Registered Types"));
        let (recent, recent_item) = submenu(mtm, ns_string!("Recent Turns"));
        let connect = item(mtm, ns_string!(""), Some(sel!(trayToggle:)), ns_string!("k"));
        let stop_now = item(mtm, ns_string!("Stop Now"), Some(sel!(trayStopNow:)), ns_string!(""));
        let stop_now_help = note(mtm, STOP_NOW_HELP, 11.0, (2.0, 4.0));
        let inspector = InspectorItem::entry(false).map(|entry| {
            item(mtm, &NSString::from_str(entry.title), Some(sel!(trayOpenInspector:)), &NSString::from_str(entry.key))
        });
        let settings = item(mtm, ns_string!("Settings…"), Some(sel!(traySettings:)), ns_string!(","));
        let quit = item(mtm, ns_string!("Quit"), Some(sel!(trayQuit:)), ns_string!("q"));
        let separator = || NSMenuItem::separatorItem(mtm);
        let (failure_separator, running_separator, context_separator) = (separator(), separator(), separator());
        let (top_separator, submenus_separator, stop_now_separator) = (separator(), separator(), separator());
        let mut rows = vec![
            &header.item,
            &top_separator,
            &failure,
            &failure_separator,
            &running_section,
            &running_separator,
            &context_section,
            &context_separator,
            &registered_item,
            &recent_item,
            &submenus_separator,
            &connect,
            &stop_now,
            &stop_now_help,
            &stop_now_separator,
        ];
        if let Some(inspector) = &inspector {
            rows.push(inspector);
        }
        rows.extend([&settings, &quit]);
        for row in rows {
            root.addItem(row);
        }
        for hidden in [
            &failure,
            &failure_separator,
            &running_section,
            &running_separator,
            &context_section,
            &context_separator,
            &registered_item,
            &recent_item,
            &submenus_separator,
            &connect,
            &stop_now,
            &stop_now_help,
            &stop_now_separator,
        ] {
            hidden.setHidden(true);
        }

        // Board Registered: the types, Background Tasks, Choose Types….
        registered.addItem(&NSMenuItem::sectionHeaderWithTitle(&NSString::from_str(ACTOR_TYPES), mtm));
        registered.addItem(&separator());
        registered.addItem(&NSMenuItem::sectionHeaderWithTitle(&NSString::from_str(BACKGROUND_TASKS.0), mtm));
        let unavailable = item(mtm, &NSString::from_str(BACKGROUND_TASKS.1), None, ns_string!(""));
        unavailable.setEnabled(false);
        registered.addItem(&unavailable);
        registered.addItem(&separator());
        let choose_types = item(mtm, &NSString::from_str(CHOOSE_TYPES), Some(sel!(traySettings:)), ns_string!(""));
        registered.addItem(&choose_types);
        // Board Running: the finished turns and the note that they are kept until the app quits.
        recent.addItem(&NSMenuItem::sectionHeaderWithTitle(&NSString::from_str(RECENT.0), mtm));
        recent.addItem(&separator());
        recent.addItem(&note(mtm, RECENT.1, 12.0, (3.0, 3.0)));

        status.setMenu(Some(&root));
        let view = Self {
            root,
            button,
            header,
            failure,
            failure_separator,
            running_section,
            running_separator,
            context_section,
            context_separator,
            registered_item,
            registered,
            recent_item,
            recent,
            submenus_separator,
            connect,
            stop_now,
            stop_now_help,
            stop_now_separator,
            choose_types,
            inspector,
            settings,
            quit,
            // objc2 exposes these selectors without version attributes. The SDK declares section headers and badges
            // in 14.0, and subtitles in 14.4. The caller already checked for 14.0.
            subtitles,
            running_rows: RefCell::default(),
            context_rows: RefCell::default(),
            registered_rows: RefCell::default(),
            recent_rows: RefCell::default(),
            shown: RefCell::new(Shown { contexts_enabled: true, ..Shown::default() }),
            icons: Icons::default(),
            status_images,
            dark: Cell::new(icons::dark(mtm)),
        };
        Some((status, view))
    }

    pub fn bind(&self, target: &Target) {
        self.root.setDelegate(Some(ProtocolObject::from_ref(target)));
        // SAFETY: The target outlives menu tracking and implements each selector with the sender type wired here.
        unsafe {
            self.header.switch.setTarget(Some(target));
            self.header.switch.setAction(Some(sel!(trayToggle:)));
            self.header.item.setTarget(Some(target));
            self.connect.setTarget(Some(target));
            self.stop_now.setTarget(Some(target));
            self.choose_types.setTarget(Some(target));
            if let Some(inspector) = &self.inspector {
                inspector.setTarget(Some(target));
            }
            self.settings.setTarget(Some(target));
            self.quit.setTarget(Some(target));
        }
    }

    /// The context a context item stands for: its represented object, never its title.
    pub fn context_of(item: &NSMenuItem) -> Option<String> {
        item.representedObject()?.downcast::<NSString>().ok().map(|name| name.to_string())
    }

    pub fn render(&self, target: &Target, model: MenuModel) {
        let mtm = target.mtm();
        let dark = icons::dark(mtm);
        let swapped = self.dark.replace(dark) != dark;
        let (title_font, subtitle_font) = (title_font(), subtitle_font());
        let rich = |item: &NSMenuItem, row: &Row| {
            let (title, subtitle) = row_text(row, self.subtitles);
            // As the boards' `text-overflow: ellipsis`: long text ends in "…" instead of widening the menu.
            let width = if row.right.is_some() { TEXT } else { WIDE_TEXT };
            item.setTitle(&NSString::from_str(&fit(&title, &title_font, width)));
            if self.subtitles {
                let subtitle = subtitle.map(|subtitle| NSString::from_str(&fit(&subtitle, &subtitle_font, width)));
                item.setSubtitle(subtitle.as_deref());
            }
            item.setBadge(row.right.as_deref().map(badge).as_deref());
            item.setImage(Some(&self.icons.image(row.icon, dark)));
            icons::show_image(item);
        };

        self.header.update(model.header, model.bar);
        // Every RefCell of the view is taken out for the render and put back at its end, so no borrow spans an AppKit
        // call: a render that AppKit re-entered would find empty state and redraw, never panic.
        let mut shown = self.shown.take();
        if shown.connected != model.connected {
            self.button.setImage(Some(&self.status_images[usize::from(model.connected)]));
            shown.connected = model.connected;
        }

        if shown.failure != model.failure || swapped {
            if let Some(row) = &model.failure {
                rich(&self.failure, row);
            }
            show(&self.failure, model.failure.is_some());
            show(&self.failure_separator, model.failure.is_some());
            shown.failure = model.failure;
        }

        let running_header = model.running.as_ref().map(|section| (section.title, section.right.clone()));
        if shown.running != running_header {
            if let Some((title, right)) = &running_header {
                self.running_section.setTitle(&NSString::from_str(title));
                self.running_section.setAttributedTitle(Some(&section_title(title)));
                self.running_section.setBadge(Some(&badge(right)));
            }
            show(&self.running_section, running_header.is_some());
            show(&self.running_separator, running_header.is_some());
            shown.running = running_header;
        }
        let start = self.root.indexOfItem(&self.running_section) + 1;
        let mut running = self.running_rows.take();
        running.sync(
            &self.root,
            start,
            model.running.map(|section| section.rows).unwrap_or_default(),
            |old, new| old.turn == new.turn,
            |_| item(mtm, ns_string!(""), None, ns_string!("")),
            |item, row| rich(item, &row.row),
        );
        if swapped {
            running.items.iter().for_each(|(row, item)| rich(item, &row.row));
        }
        self.running_rows.replace(running);

        let contexts = model.contexts.unwrap_or_default();
        show(&self.context_section, !contexts.is_empty());
        show(&self.context_separator, !contexts.is_empty());
        let start = self.root.indexOfItem(&self.context_section) + 1;
        let mut rows = self.context_rows.take();
        rows.sync(
            &self.root,
            start,
            contexts,
            |old, new| old.name == new.name,
            |row| {
                let item = item(mtm, ns_string!(""), Some(sel!(trayPickContext:)), ns_string!(""));
                // SAFETY: The target implements trayPickContext: with an NSMenuItem sender and owns this menu. The
                // represented object is an NSString, which `context_of` expects.
                unsafe {
                    item.setTarget(Some(target));
                    item.setRepresentedObject(Some(&NSString::from_str(&row.name)));
                }
                item.setEnabled(model.contexts_enabled);
                item
            },
            |item, row| rich(item, &row.row),
        );
        if swapped {
            rows.items.iter().for_each(|(row, item)| rich(item, &row.row));
        }
        if shown.contexts_enabled != model.contexts_enabled {
            for (_, item) in &rows.items {
                item.setEnabled(model.contexts_enabled);
            }
            shown.contexts_enabled = model.contexts_enabled;
        }
        self.context_rows.replace(rows);

        let submenus = model.submenus;
        for item in [&self.registered_item, &self.recent_item, &self.submenus_separator] {
            show(item, submenus.is_some());
        }
        let (types, recent, registered_right, recent_right) = submenus
            .map(|submenus| (submenus.types, submenus.recent, submenus.registered_right, submenus.recent_right))
            .unwrap_or_default();
        if shown.registered_right != registered_right {
            self.registered_item.setBadge(registered_right.as_deref().map(badge).as_deref());
            shown.registered_right = registered_right;
        }
        if shown.recent_right != recent_right {
            self.recent_item.setBadge(recent_right.as_deref().map(badge).as_deref());
            shown.recent_right = recent_right;
        }
        let mut registered_rows = self.registered_rows.take();
        registered_rows.sync(
            &self.registered,
            1,
            types,
            |old, new| old.name == new.name,
            |row| item(mtm, &NSString::from_str(&fit(row.name, &title_font, TEXT)), None, ns_string!("")),
            |item, row| {
                item.setState(mark(row.runs));
                item.setBadge(row.right.as_deref().map(badge).as_deref());
            },
        );
        self.registered_rows.replace(registered_rows);
        let mut recent_rows = self.recent_rows.take();
        recent_rows.sync(
            &self.recent,
            1,
            recent,
            |old, new| old.turn == new.turn,
            |_| item(mtm, ns_string!(""), None, ns_string!("")),
            |item, row| rich(item, &row.row),
        );
        if swapped {
            recent_rows.items.iter().for_each(|(row, item)| rich(item, &row.row));
        }
        self.recent_rows.replace(recent_rows);

        if shown.connect != model.connect {
            if let Some(title) = &model.connect {
                self.connect.setTitle(&NSString::from_str(&fit(title, &title_font, PLAIN_TEXT)));
            }
            show(&self.connect, model.connect.is_some());
            shown.connect = model.connect;
        }
        for item in [&self.stop_now, &self.stop_now_help, &self.stop_now_separator] {
            show(item, model.stop_now);
        }
        if let Some(item) = &self.inspector {
            show(item, model.inspector.is_some());
            if let Some(entry) = model.inspector {
                item.setEnabled(entry.enabled);
            }
        }
        show(&self.settings, model.settings);
        let quit = (model.quit, model.quit_enabled);
        if shown.quit != quit {
            self.quit.setTitle(&NSString::from_str(&fit(&quit.0, &title_font, PLAIN_TEXT)));
            self.quit.setEnabled(quit.1);
            shown.quit = quit;
        }
        self.shown.replace(shown);
    }
}

/// The longest text of a row with a badge, and of one without, and of a plain item, in points: with the icon, the
/// badge and the margins they keep a row within the boards' 300 pt menu.
const TEXT: f64 = 170.0;
const WIDE_TEXT: f64 = 210.0;
const PLAIN_TEXT: f64 = 230.0;

fn title_font() -> Retained<NSFont> {
    NSFont::menuFontOfSize(0.0)
}

fn subtitle_font() -> Retained<NSFont> {
    NSFont::systemFontOfSize(NSFont::smallSystemFontSize())
}

fn attributed(text: &str, font: &NSFont, color: Option<&NSColor>) -> Retained<NSAttributedString> {
    // SAFETY: AppKit's attribute names are constants valid for the life of the process.
    let (font_key, color_key) = unsafe { (NSFontAttributeName, NSForegroundColorAttributeName) };
    let mut keys = vec![font_key];
    let mut values: Vec<&AnyObject> = vec![font.as_ref()];
    if let Some(color) = color {
        keys.push(color_key);
        values.push(color.as_ref());
    }
    let attributes = NSDictionary::from_slices(&keys, &values);
    // SAFETY: The attributes map NSAttributedStringKeys to the value types AppKit expects: an NSFont and an NSColor.
    unsafe {
        NSAttributedString::initWithString_attributes(
            NSAttributedString::alloc(),
            &NSString::from_str(text),
            Some(&attributes),
        )
    }
}

/// `text`, cut with a trailing "…" to fit `width` points in `font`.
fn fit(text: &str, font: &NSFont, width: f64) -> String {
    let fits = |text: &str| attributed(text, font, None).size().width <= width;
    if fits(text) {
        return text.to_owned();
    }
    let chars: Vec<char> = text.chars().collect();
    let cut = |count: usize| chars[..count].iter().collect::<String>().trim_end().to_owned() + "…";
    // The longest prefix that fits with the ellipsis, by bisection.
    let (mut short, mut long) = (0, chars.len());
    while short < long {
        let middle = (short + long).div_ceil(2);
        if fits(&cut(middle)) { short = middle } else { long = middle - 1 }
    }
    cut(short)
}

/// A section header as the boards draw it: 11 pt, semibold, in the secondary colour.
fn section_title(text: &str) -> Retained<NSAttributedString> {
    // SAFETY: NSFontWeightSemibold is AppKit's constant.
    let font = NSFont::systemFontOfSize_weight(11.0, unsafe { NSFontWeightSemibold });
    attributed(text, &font, Some(&NSColor::secondaryLabelColor()))
}

/// Shows or hides an item, touching AppKit only on a change.
fn show(item: &NSMenuItem, visible: bool) {
    if item.isHidden() == visible {
        item.setHidden(!visible);
    }
}

/// The text on the right of a row: native menus draw it only as a badge (the design's first departure).
fn badge(text: &str) -> Retained<NSMenuItemBadge> {
    NSMenuItemBadge::initWithString(NSMenuItemBadge::alloc(), &NSString::from_str(text))
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

/// A row of secondary text that wraps, as the boards' disabled notes: the help under Stop Now and the note at the end
/// of Recent Turns. A view, so the text wraps at the menu's width; it has no action.
fn note(mtm: MainThreadMarker, text: &str, size: f64, (top, bottom): (f64, f64)) -> Retained<NSMenuItem> {
    let label = NSTextField::wrappingLabelWithString(&NSString::from_str(text), mtm);
    label.setFont(Some(&NSFont::systemFontOfSize(size)));
    label.setTextColor(Some(&NSColor::secondaryLabelColor()));
    let width = WIDTH - 2.0 * PADDING;
    label.setPreferredMaxLayoutWidth(width);
    let height = label.sizeThatFits(NSSize::new(width, f64::MAX)).height.ceil();
    label.setFrame(rect(PADDING, bottom, width, height));
    label.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
    let view = NSView::initWithFrame(NSView::alloc(mtm), rect(0.0, 0.0, WIDTH, top + height + bottom));
    view.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
    view.addSubview(&label);
    let item = item(mtm, &NSString::from_str(text), None, ns_string!(""));
    item.setView(Some(&view));
    item.setEnabled(false);
    item
}

fn mark(marked: bool) -> NSControlStateValue {
    if marked { NSControlStateValueOn } else { NSControlStateValueOff }
}

/// The header's height without and with the disconnecting bar (boards Main and Disconnecting).
const HEADER: f64 = 44.0;
const HEADER_BAR: f64 = 33.0;

/// The header: the app's title over the connection, the switch, and while disconnecting the grace period's bar with
/// its text, all in one custom view. The bar uses the system's progress style (the design's fourth departure).
struct HeaderView {
    item: Retained<NSMenuItem>,
    view: Retained<NSView>,
    title: Retained<NSTextField>,
    subtitle: Retained<NSTextField>,
    switch: Retained<NSSwitch>,
    bar: Retained<NSProgressIndicator>,
    bar_text: Retained<NSTextField>,
    bar_left: Retained<NSTextField>,
    shown: RefCell<Option<(Header, Option<Bar>)>>,
}

impl HeaderView {
    fn new(mtm: MainThreadMarker) -> Self {
        let view = NSView::initWithFrame(NSView::alloc(mtm), rect(0.0, 0.0, WIDTH, HEADER));
        view.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
        let label = |size: f64, secondary: bool| {
            let field = NSTextField::labelWithString(ns_string!(""), mtm);
            field.setFont(Some(&NSFont::systemFontOfSize(size)));
            if secondary {
                field.setTextColor(Some(&NSColor::secondaryLabelColor()));
            }
            field.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
            field.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
            view.addSubview(&field);
            field
        };
        let title = label(13.0, false);
        title.setFont(Some(&NSFont::boldSystemFontOfSize(13.0)));
        let subtitle = label(11.0, true);
        let bar_text = label(11.0, true);
        let bar_left = label(11.0, true);
        bar_left.setAlignment(NSTextAlignment::Right);
        bar_left.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinXMargin);
        let switch = NSSwitch::initWithFrame(NSSwitch::alloc(mtm), rect(0.0, 0.0, 38.0, 22.0));
        switch.setAccessibilityLabel(Some(ns_string!("Worker connection")));
        switch.setAutoresizingMask(NSAutoresizingMaskOptions::ViewMinXMargin);
        view.addSubview(&switch);
        let bar = NSProgressIndicator::initWithFrame(NSProgressIndicator::alloc(mtm), rect(0.0, 0.0, 0.0, 0.0));
        bar.setStyle(NSProgressIndicatorStyle::Bar);
        bar.setControlSize(NSControlSize::Small);
        bar.setIndeterminate(false);
        bar.setMaxValue(1.0);
        bar.setAutoresizingMask(NSAutoresizingMaskOptions::ViewWidthSizable);
        view.addSubview(&bar);
        // AppKit gives the custom view mouse events and handles keyboard selection on the item itself.
        let item = item(mtm, ns_string!("Worker Connection"), Some(sel!(trayToggle:)), ns_string!(""));
        let header = Self { item, view, title, subtitle, switch, bar, bar_text, bar_left, shown: RefCell::default() };
        header.layout(false);
        header.item.setView(Some(&header.view));
        header
    }

    /// Places the subviews for a header with or without the bar, at the view's current width.
    fn layout(&self, with_bar: bool) {
        let width = self.view.frame().size.width;
        let bar = if with_bar { HEADER_BAR } else { 0.0 };
        self.view.setFrameSize(NSSize::new(width, HEADER + bar));
        let text = width - 2.0 * PADDING - 48.0;
        self.title.setFrame(rect(PADDING, bar + 22.0, text, 17.0));
        self.subtitle.setFrame(rect(PADDING, bar + 7.0, text, 15.0));
        self.switch.setFrame(rect(width - PADDING - 38.0, bar + 11.0, 38.0, 22.0));
        let inner = width - 2.0 * PADDING;
        self.bar.setFrame(rect(PADDING, 22.0, inner, 10.0));
        self.bar_text.setFrame(rect(PADDING, 6.0, inner - 80.0, 15.0));
        self.bar_left.setFrame(rect(width - PADDING - 80.0, 6.0, 80.0, 15.0));
        for view in [&*self.bar as &NSView, &self.bar_text, &self.bar_left] {
            view.setHidden(!with_bar);
        }
    }

    fn update(&self, header: Header, bar: Option<Bar>) {
        // Taken out, so no borrow spans the AppKit calls below; put back at the end.
        let shown = self.shown.take();
        let previous = shown.as_ref();
        if previous.map(|(header, _)| header) != Some(&header) {
            self.title.setStringValue(&NSString::from_str(&header.title));
            self.subtitle.setStringValue(&NSString::from_str(&header.subtitle));
            self.switch.setEnabled(header.enabled);
            self.item.setEnabled(header.enabled);
        }
        if previous.map(|(_, bar)| bar.is_some()) != Some(bar.is_some()) {
            self.layout(bar.is_some());
            // Hand the view to the item again so an open menu takes its new height.
            self.item.setView(Some(&self.view));
        }
        if let Some(next) = &bar
            && previous.and_then(|(_, bar)| bar.as_ref()) != Some(next)
        {
            self.bar.setDoubleValue(next.fraction);
            self.bar.setAccessibilityLabel(Some(&NSString::from_str(&next.label)));
            self.bar_text.setStringValue(&NSString::from_str(next.text));
            self.bar_left.setStringValue(&NSString::from_str(next.left.as_deref().unwrap_or_default()));
        }
        let state = mark(header.on);
        if self.switch.state() != state {
            self.switch.setState(state);
        }
        self.shown.replace(Some((header, bar)));
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
