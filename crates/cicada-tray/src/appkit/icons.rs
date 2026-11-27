//! The boards' icons: each row kind's glyph in a tinted 26 pt circle, per appearance, and the status item's hexagon.

use std::cell::RefCell;
use std::collections::HashMap;

use block2::RcBlock;
use objc2::rc::Retained;
use objc2::runtime::Bool;
use objc2::{MainThreadMarker, msg_send, sel};
use objc2_app_kit::{
    NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSApplication, NSBezierPath, NSColor, NSImage, NSLineCapStyle,
    NSLineJoinStyle, NSMenuItem,
};
use objc2_foundation::{NSArray, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString};

use crate::menu::Icon;

/// The circle's diameter, as on the boards.
const CIRCLE: f64 = 26.0;

/// A drawing step of a glyph, in the boards' 16-unit SVG coordinates (y down).
#[derive(Clone, Copy)]
enum Op {
    Move(f64, f64),
    Line(f64, f64),
    Close,
    /// Stroked, and also filled when `true`, as an SVG circle with a fill under the glyph's stroke.
    Circle(f64, f64, f64, bool),
    RoundRect(f64, f64, f64, f64, f64),
}

/// A glyph: drawn `size` points wide, with the SVG's stroke width.
struct Glyph {
    size: f64,
    width: f64,
    ops: &'static [Op],
}

use Op::{Circle, Close, Line, Move, RoundRect};

/// The pulse of a running turn.
const PULSE: Glyph = Glyph {
    size: 14.0,
    width: 1.7,
    ops: &[Move(1.5, 8.0), Line(4.3, 8.0), Line(6.0, 3.5), Line(9.2, 12.5), Line(10.9, 8.0), Line(14.5, 8.0)],
};
/// The clock of a finishing turn.
const CLOCK: Glyph = Glyph {
    size: 14.0,
    width: 1.7,
    ops: &[Circle(8.0, 8.0, 5.5, false), Move(8.0, 5.0), Line(8.0, 8.0), Line(10.0, 9.5)],
};
/// The server of a context.
const SERVER: Glyph = Glyph {
    size: 14.0,
    width: 1.5,
    ops: &[RoundRect(2.5, 2.75, 11.0, 4.5, 1.2), RoundRect(2.5, 8.75, 11.0, 4.5, 1.2)],
};
/// The check of a done turn.
const CHECK: Glyph = Glyph { size: 13.0, width: 2.2, ops: &[Move(3.5, 8.5), Line(6.5, 11.5), Line(12.5, 4.5)] };
/// The cross of a failed or cancelled turn.
const CROSS: Glyph =
    Glyph { size: 12.0, width: 2.2, ops: &[Move(4.5, 4.5), Line(11.5, 11.5), Move(11.5, 4.5), Line(4.5, 11.5)] };
/// The exclamation mark of the connection failure.
const ALERT: Glyph =
    Glyph { size: 14.0, width: 1.8, ops: &[Move(8.0, 4.0), Line(8.0, 9.0), Circle(8.0, 12.0, 0.6, true)] };
const HEXAGON: [Op; 7] =
    [Move(8.0, 1.75), Line(13.5, 4.9), Line(13.5, 11.1), Line(8.0, 14.25), Line(2.5, 11.1), Line(2.5, 4.9), Close];
/// The status item, connected: the hexagon with a filled dot.
const CONNECTED: Glyph = Glyph {
    size: 16.0,
    width: 1.5,
    ops: &[
        HEXAGON[0],
        HEXAGON[1],
        HEXAGON[2],
        HEXAGON[3],
        HEXAGON[4],
        HEXAGON[5],
        HEXAGON[6],
        Circle(8.0, 8.0, 2.0, true),
    ],
};
/// The status item otherwise: the hexagon with a hollow dot.
const NOT_CONNECTED: Glyph = Glyph {
    size: 16.0,
    width: 1.5,
    ops: &[
        HEXAGON[0],
        HEXAGON[1],
        HEXAGON[2],
        HEXAGON[3],
        HEXAGON[4],
        HEXAGON[5],
        HEXAGON[6],
        Circle(8.0, 8.0, 2.0, false),
    ],
};

/// An sRGB colour with alpha.
type Rgba = (f64, f64, f64, f64);

fn hex(rgb: u32) -> Rgba {
    let channel = |shift: u32| f64::from((rgb >> shift) & 0xff) / 255.0;
    (channel(16), channel(8), channel(0), 1.0)
}

fn rgba(r: u8, g: u8, b: u8, a: f64) -> Rgba {
    (f64::from(r) / 255.0, f64::from(g) / 255.0, f64::from(b) / 255.0, a)
}

/// The circle's fill, the glyph's colour and the glyph, from the boards' `.t` (light) and `.t.dark` colours.
fn style(icon: Icon, dark: bool) -> (Rgba, Rgba, &'static Glyph) {
    let pick = |light: Rgba, night: Rgba| if dark { night } else { light };
    let green = (pick(rgba(52, 199, 89, 0.18), rgba(48, 209, 88, 0.2)), pick(hex(0x34c759), hex(0x30d158)));
    let red = (pick(rgba(255, 59, 48, 0.14), rgba(255, 69, 58, 0.2)), pick(hex(0xff3b30), hex(0xff453a)));
    let orange = (pick(rgba(255, 149, 0, 0.18), rgba(255, 159, 10, 0.2)), pick(hex(0xff9500), hex(0xff9f0a)));
    let grey = pick(rgba(0, 0, 0, 0.08), rgba(255, 255, 255, 0.13));
    let label = pick(rgba(0, 0, 0, 0.86), rgba(255, 255, 255, 0.88));
    let secondary = pick(rgba(0, 0, 0, 0.52), rgba(255, 255, 255, 0.56));
    let blue = pick(hex(0x007aff), hex(0x0a84ff));
    match icon {
        Icon::Running => (green.0, green.1, &PULSE),
        Icon::Finishing => (orange.0, orange.1, &CLOCK),
        Icon::Context { selected: true } => (blue, hex(0xffffff), &SERVER),
        Icon::Context { selected: false } => (grey, label, &SERVER),
        Icon::Done => (green.0, green.1, &CHECK),
        Icon::Failed => (red.0, red.1, &CROSS),
        Icon::Cancelled => (grey, secondary, &CROSS),
        Icon::Alert => (red.0, red.1, &ALERT),
    }
}

fn color((r, g, b, a): Rgba) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(r, g, b, a)
}

/// Draws `glyph` centred in a square of `side` points, in a flipped context.
fn draw(glyph: &Glyph, side: f64, ink: Rgba) {
    let scale = glyph.size / 16.0;
    let offset = (side - glyph.size) / 2.0;
    let point = |x: f64, y: f64| NSPoint::new(offset + x * scale, offset + y * scale);
    let ink = color(ink);
    ink.setStroke();
    ink.setFill();
    let path = NSBezierPath::bezierPath();
    let stroke = |path: &NSBezierPath| {
        path.setLineWidth(glyph.width * scale);
        path.setLineCapStyle(NSLineCapStyle::Round);
        path.setLineJoinStyle(NSLineJoinStyle::Round);
        path.stroke();
    };
    for op in glyph.ops {
        match *op {
            Move(x, y) => path.moveToPoint(point(x, y)),
            Line(x, y) => path.lineToPoint(point(x, y)),
            Close => path.closePath(),
            Circle(x, y, r, filled) => {
                let origin = point(x - r, y - r);
                let oval = NSBezierPath::bezierPathWithOvalInRect(NSRect::new(
                    origin,
                    NSSize::new(2.0 * r * scale, 2.0 * r * scale),
                ));
                if filled {
                    oval.fill();
                }
                stroke(&oval);
            }
            RoundRect(x, y, w, h, r) => {
                let rect = NSRect::new(point(x, y), NSSize::new(w * scale, h * scale));
                stroke(&NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(rect, r * scale, r * scale));
            }
        }
    }
    stroke(&path);
}

fn image(side: f64, paint: impl Fn() + 'static) -> Retained<NSImage> {
    let handler = RcBlock::new(move |_: NSRect| {
        paint();
        Bool::YES
    });
    NSImage::imageWithSize_flipped_drawingHandler(NSSize::new(side, side), true, &handler)
}

/// The row images, made once per icon and appearance.
#[derive(Default)]
pub(super) struct Icons {
    made: RefCell<HashMap<(Icon, bool), Retained<NSImage>>>,
}

impl Icons {
    /// The icon's image for the light or dark appearance: not a template, so it keeps the boards' colours.
    pub fn image(&self, icon: Icon, dark: bool) -> Retained<NSImage> {
        self.made
            .borrow_mut()
            .entry((icon, dark))
            .or_insert_with(|| {
                let (fill, ink, glyph) = style(icon, dark);
                let image = image(CIRCLE, move || {
                    color(fill).setFill();
                    NSBezierPath::bezierPathWithOvalInRect(NSRect::new(NSPoint::ZERO, NSSize::new(CIRCLE, CIRCLE)))
                        .fill();
                    draw(glyph, CIRCLE, ink);
                });
                image.setTemplate(false);
                image
            })
            .clone()
    }
}

/// The status item's template image: the filled dot when connected, else the hollow one at 55% opacity.
pub(super) fn status_image(connected: bool, label: &NSString) -> Retained<NSImage> {
    let image = image(16.0, move || {
        let (glyph, alpha) = if connected { (&CONNECTED, 1.0) } else { (&NOT_CONNECTED, 0.55) };
        draw(glyph, 16.0, (0.0, 0.0, 0.0, alpha));
    });
    image.setTemplate(true);
    image.setAccessibilityDescription(Some(label));
    image
}

/// Whether the app shows the dark appearance now.
pub(super) fn dark(mtm: MainThreadMarker) -> bool {
    let appearance = NSApplication::sharedApplication(mtm).effectiveAppearance();
    // SAFETY: Both names are AppKit's constants, valid for the life of the process.
    let names = unsafe { NSArray::from_slice(&[NSAppearanceNameAqua, NSAppearanceNameDarkAqua]) };
    // SAFETY: As above.
    let dark = unsafe { NSAppearanceNameDarkAqua };
    appearance.bestMatchFromAppearancesWithNames(&names).is_some_and(|name| &*name == dark)
}

/// Asks AppKit to show the item's image where it hides images by default (macOS 27), with
/// `preferredImageVisibility = .visible`; earlier systems lack the property and always show it.
pub(super) fn show_image(item: &NSMenuItem) {
    if item.respondsToSelector(sel!(setPreferredImageVisibility:)) {
        // SAFETY: The selector exists, checked above, and takes an NSInteger: NSMenuItemImageVisibilityVisible is 1.
        let () = unsafe { msg_send![item, setPreferredImageVisibility: 1isize] };
    }
}
