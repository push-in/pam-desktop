//! Paragraph typesetting with TextKit, shared by measurement, drawing, hit
//! testing and selection so that all four always agree.
//!
//! One layout manager serves every paragraph: a view keeps only its
//! attributed string and the paragraph is laid out again when it is
//! measured, painted or hit tested. Views cache their pixels in their layer,
//! so this happens when the text or the box changes, not on every frame.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};

use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::runtime::{Bool, NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{AnyThread, DefinedClass, MainThreadMarker, define_class, msg_send};
use objc2_app_kit::{
    NSAttributedStringNSStringDrawing, NSBackgroundColorAttributeName,
    NSBaselineOffsetAttributeName, NSColor, NSFont, NSFontAttributeName, NSFontDescriptor,
    NSFontDescriptorSymbolicTraits, NSFontDescriptorSystemDesignRounded,
    NSFontDescriptorSystemDesignSerif, NSFontFeatureSettingsAttribute, NSFontManager,
    NSFontTraitMask, NSFontVariationAttribute, NSForegroundColorAttributeName, NSKernAttributeName,
    NSLayoutManager, NSLayoutManagerDelegate, NSLineBreakMode, NSMutableParagraphStyle,
    NSParagraphStyleAttributeName, NSStrikethroughStyleAttributeName, NSTextAlignment,
    NSTextContainer, NSTextStorage, NSUnderlineStyleAttributeName, NSWritingDirection,
};
use objc2_core_foundation::{CGAffineTransform, CGFloat, CGPoint, CGRect, CGSize};
use objc2_core_graphics::CGContext;
use objc2_core_text::{
    CTFont, CTFontOrientation, kCTFontOpenTypeFeatureTag, kCTFontOpenTypeFeatureValue,
};
use objc2_foundation::{
    NSArray, NSAttributedString, NSAttributedStringKey, NSDictionary, NSMutableAttributedString,
    NSMutableDictionary, NSNumber, NSRange, NSString, NSUInteger,
};
use pam_desktop_ui::geometry::Size;
use pam_desktop_ui::style::{Border, Decoration, Radii};
use pam_desktop_ui::text::{
    Ellipsize, FontSpec, InlineBox, SPACER_OPEN, SpacerMap, TextAlign, TextBlock, TextMetrics,
    TextShadow, TextWrap,
};
use std::ptr::NonNull;

use crate::util::{cg_color, ns_color, ns_string, utf16_len};

/// Stands for "no width limit": far wider than any line of text.
const UNBOUNDED: CGFloat = 10_000_000.0;
/// Fonts kept resolved; the cache is dropped whole when it outgrows this.
const FONT_CACHE_LIMIT: usize = 256;
/// Measured paragraphs kept for the next measurement of the same text.
const MEASURE_CACHE_LIMIT: usize = 32;

fn any<T: AsRef<AnyObject> + ?Sized>(value: &T) -> &AnyObject {
    value.as_ref()
}

/// A paragraph ready to be laid out.
pub struct Paragraph {
    string: Retained<NSAttributedString>,
    /// Identifies the paragraph in the layout cache.
    serial: u64,
    max_lines: u32,
    ellipsize: Ellipsize,
    wrap: TextWrap,
    /// CSS `-webkit-font-smoothing: antialiased`.
    antialiased: bool,
    /// Where Chromium puts the first baseline below the line top: each
    /// font's ascent and descent rounded to whole pixels and
    /// `floor(half-leading)` above, so baselines fall on whole pixels.
    chromium_baseline: f32,
    shadow: Option<TextShadow>,
    /// UTF-16 length of the authored text.
    length: usize,
    /// The block had no text: a space stands in, so the line has its height.
    empty: bool,
    /// Paragraph offsets to and from the laid-out string, which holds one
    /// spacer character per inline box edge.
    map: SpacerMap,
    /// UTF-16 length of the laid-out string (text plus spacers).
    layout_length: usize,
    /// Inline boxes (`TextSpanBoxes`): the laid-out range from the opening
    /// spacer through the closing one, and the font of the box's text.
    boxes: Vec<(NSRange, InlineBox, Retained<NSFont>)>,
    /// Exact minus snapped top of the paragraph in device pixels, and the
    /// device scale, for the next `draw` (see `set_line_phase`).
    line_phase: Cell<(f64, f64)>,
    /// Inline-blocks taller than the line: which lines keep the base height.
    line_rule: Option<LineRule>,
}

/// Closes an inline box in the laid-out string. Like the shared
/// `SPACER_CLOSE` (WORD JOINER) it never breaks from the text on either
/// side, but it is a real glyph, so TextKit applies its kern.
const LAYOUT_SPACER_CLOSE: char = '\u{00A0}';

/// Advance of one BMP character in a font, in points.
fn advance(font: &NSFont, character: char) -> CGFloat {
    let mut unit = [0_u16; 2];
    if character.encode_utf16(&mut unit).len() != 1 {
        return 0.0;
    }
    // SAFETY: `NSFont` is toll-free bridged with `CTFont`; both pointers
    // are valid for one element during the calls.
    unsafe {
        let font = &*std::ptr::from_ref(font).cast::<CTFont>();
        let mut glyph = 0_u16;
        if !font.glyphs_for_characters(
            std::ptr::NonNull::from(&mut unit[0]),
            std::ptr::NonNull::from(&mut glyph),
            1,
        ) {
            return 0.0;
        }
        font.advances_for_glyphs(
            CTFontOrientation::Horizontal,
            std::ptr::NonNull::from(&mut glyph),
            std::ptr::null_mut(),
            1,
        )
    }
}

/// Blink's text-field metrics of the block's first font, as Chromium's Mac
/// port computes them (`SimpleFontData::PlatformInit`): the average is the
/// advance of `x` from the font's own `hmtx` (Skia's Core Text glyph widths
/// carry no size-specific tracking), the maximum the ascent rounded like
/// Blink (with its Times/Helvetica/Courier adjustment).
pub fn control_metrics(block: &TextBlock) -> Option<pam_desktop_ui::text::CharMetrics> {
    let run = block.runs.first()?;
    let font = font(&run.font);
    // SAFETY: `NSFont` is toll-free bridged with `CTFont`; the glyph and
    // advance buffers are valid for one element during the calls.
    let average = unsafe {
        let font = &*std::ptr::from_ref(&*font).cast::<CTFont>();
        let mut unit = [u16::from(b'x')];
        let mut glyph = 0_u16;
        if !font.glyphs_for_characters(
            std::ptr::NonNull::from(&mut unit[0]),
            std::ptr::NonNull::from(&mut glyph),
            1,
        ) {
            return None;
        }
        let graphics = font.graphics_font(std::ptr::null_mut());
        let units = objc2_core_graphics::CGFont::units_per_em(Some(&graphics));
        let mut advance = 0_i32;
        if units <= 0
            || !objc2_core_graphics::CGFont::glyph_advances(
                Some(&graphics),
                std::ptr::NonNull::from(&mut glyph),
                1,
                std::ptr::NonNull::from(&mut advance),
            )
        {
            return None;
        }
        advance as f32 / units as f32 * font.size() as f32
    };
    let mut ascent = font.ascender().round() as f32;
    let descent = (-font.descender()).round() as f32;
    let family = font
        .familyName()
        .map(|name| name.to_string())
        .unwrap_or_default();
    if matches!(family.as_str(), "Times" | "Helvetica" | "Courier") {
        ascent += ((ascent + descent) * 0.15 + 0.5).floor();
    }
    Some(pam_desktop_ui::text::CharMetrics {
        average,
        maximum: ascent,
    })
}

#[derive(Clone, Eq, Hash, PartialEq)]
struct FontKey {
    family: String,
    size: u32,
    weight: u16,
    italic: bool,
    features: String,
}

define_class!(
    // SAFETY: `NSTextContainer` may be subclassed to shape line fragments;
    // the instance variable is a plain `Cell` and the class has no `Drop`.
    #[unsafe(super(NSTextContainer))]
    #[ivars = Cell<bool>]
    struct WordContainer;

    impl WordContainer {
        /// CSS `overflow-wrap: normal`: a word wider than the column is never
        /// broken. TextKit breaks it between characters, so while the loaded
        /// paragraph has such a word, a line that starts with one is made as
        /// wide as the word: it overflows the column alone, like a browser.
        #[unsafe(method(lineFragmentRectForProposedRect:atIndex:writingDirection:remainingRect:))]
        fn line_fragment(
            &self,
            proposed: CGRect,
            index: NSUInteger,
            direction: NSWritingDirection,
            remaining: *mut CGRect,
        ) -> CGRect {
            // SAFETY: the arguments are forwarded unchanged to the superclass.
            let mut rect: CGRect = unsafe {
                msg_send![super(self), lineFragmentRectForProposedRect: proposed, atIndex: index, writingDirection: direction, remainingRect: remaining]
            };
            // SAFETY: the container belongs to the typesetter's layout
            // manager, which keeps its text storage alive.
            let storage = || unsafe { self.layoutManager().and_then(|manager| manager.textStorage()) };
            if self.ivars().get()
                && let Some(storage) = storage()
            {
                let word = word_at(&storage.string(), index);
                if word.length > 0 {
                    let width = storage.attributedSubstringFromRange(word).size().width.ceil();
                    if width > rect.size.width {
                        rect.size.width = width;
                    }
                }
            }
            rect
        }

        #[unsafe(method(isSimpleRectangularTextContainer))]
        fn simple(&self) -> bool {
            !self.ivars().get()
        }
    }
);

impl WordContainer {
    fn new() -> Retained<Self> {
        let this = Self::alloc().set_ivars(Cell::new(false));
        let size = CGSize::new(UNBOUNDED, UNBOUNDED);
        // SAFETY: `initWithSize:` is the designated initializer of `NSTextContainer`.
        unsafe { msg_send![super(this), initWithSize: size] }
    }
}

/// The unbreakable run starting at `index`: up to the next space, or just
/// past the next hyphen or slash (where browsers may break too).
/// Lines of a paragraph that holds inline-blocks: the paragraph style makes
/// every line as tall as the tallest box, while CSS grows only the lines a
/// box sits on.
#[derive(Clone, Debug, PartialEq)]
struct LineRule {
    /// The height of a line without a box (the block's line height).
    base: CGFloat,
    /// Plain lines' baseline minus the boxed lines' one, below the top.
    baseline: CGFloat,
    /// Laid-out character ranges of the inline-blocks.
    boxes: Vec<NSRange>,
}

define_class!(
    // SAFETY: `NSObject` may be subclassed; the delegate method keeps the
    // contract of `NSLayoutManagerDelegate` (it only writes through the
    // pointers TextKit passes). The ivar is a plain `RefCell`.
    #[unsafe(super(NSObject))]
    #[ivars = RefCell<Option<LineRule>>]
    struct LineHeights;

    unsafe impl NSObjectProtocol for LineHeights {}

    unsafe impl NSLayoutManagerDelegate for LineHeights {
        #[unsafe(method(layoutManager:shouldSetLineFragmentRect:lineFragmentUsedRect:baselineOffset:inTextContainer:forGlyphRange:))]
        fn should_set_line_fragment(
            &self,
            manager: &NSLayoutManager,
            rect: NonNull<CGRect>,
            used: NonNull<CGRect>,
            baseline: NonNull<CGFloat>,
            _container: &NSTextContainer,
            glyphs: NSRange,
        ) -> Bool {
            let rule = self.ivars().borrow();
            let Some(rule) = rule.as_ref() else {
                return Bool::NO;
            };
            // SAFETY: no actual range is asked for.
            let characters = unsafe {
                manager.characterRangeForGlyphRange_actualGlyphRange(glyphs, std::ptr::null_mut())
            };
            let (start, end) = (characters.location, characters.location + characters.length);
            if rule.boxes.iter().any(|range| range.location < end && start < range.location + range.length) {
                return Bool::NO;
            }
            // SAFETY: TextKit passes valid in-out pointers for this call.
            unsafe {
                (*rect.as_ptr()).size.height = rule.base;
                (*used.as_ptr()).size.height = rule.base;
                *baseline.as_ptr() += rule.baseline;
            }
            Bool::YES
        }
    }
);

impl LineHeights {
    fn new() -> Retained<Self> {
        let this = Self::alloc().set_ivars(RefCell::new(None));
        // SAFETY: `init` is the designated initializer of `NSObject`.
        unsafe { msg_send![super(this), init] }
    }
}

define_class!(
    // SAFETY: `NSLayoutManager` may be subclassed; the override forwards to
    // the superclass with the same glyphs and font and adjusted positions.
    #[unsafe(super(NSLayoutManager, NSObject))]
    #[ivars = ()]
    struct GlyphPlacer;

    impl GlyphPlacer {
        /// While a paragraph draws Chromium's masks (`PLACE_GLYPHS`), draws
        /// the run with `glyphs::draw`. Glyphs that pipeline leaves to Core
        /// Graphics are placed like Skia's subpixel text: along the baseline
        /// on a quarter of a device pixel, `floor(x + 1/8)` in quarters, the
        /// baseline on a whole pixel, `floor(y + 1/2)`; Core Graphics draws
        /// them exactly there (the paragraph turns its own quantization off:
        /// it keeps fewer positions above 11 px).
        #[unsafe(method(showCGGlyphs:positions:count:font:textMatrix:attributes:inContext:))]
        fn show_glyphs(
            &self,
            glyphs: *const objc2_core_graphics::CGGlyph,
            positions: *const CGPoint,
            count: objc2_foundation::NSInteger,
            font: &NSFont,
            matrix: CGAffineTransform,
            attributes: &NSDictionary<NSAttributedStringKey, AnyObject>,
            context: &CGContext,
        ) {
            let mut placed = Vec::new();
            let mut positions = positions;
            if let Some(smooth) = PLACE_GLYPHS.get()
                && count > 0
                && !positions.is_null()
                && !glyphs.is_null()
            {
                // SAFETY: TextKit passes `count` glyphs and positions.
                let (given, ids) = unsafe {
                    (
                        std::slice::from_raw_parts(positions, count as usize),
                        std::slice::from_raw_parts(glyphs, count as usize),
                    )
                };
                // SAFETY: the attribute name is an immutable string exported by AppKit.
                let color = attributes
                    .objectForKey(unsafe { NSForegroundColorAttributeName })
                    .and_then(|color| color.downcast::<NSColor>().ok());
                if crate::glyphs::draw(context, font, ids, given, color.as_deref(), smooth) {
                    return;
                }
                let device = CGContext::user_space_to_device_space_transform(Some(context));
                if let Some(snapped) = quarter_positions(given, &device) {
                    placed = snapped;
                    positions = placed.as_ptr();
                }
            }
            // SAFETY: forwards the same selector with valid arguments; the
            // adjusted positions outlive the call.
            unsafe {
                let _: () = msg_send![super(self), showCGGlyphs: glyphs, positions: positions, count: count, font: font, textMatrix: matrix, attributes: attributes, inContext: context];
            }
            drop(placed);
        }
    }
);

impl GlyphPlacer {
    fn new() -> Retained<Self> {
        let this = Self::alloc().set_ivars(());
        // SAFETY: `init` is the designated initializer of `NSLayoutManager`.
        unsafe { msg_send![super(this), init] }
    }
}

thread_local! {
    /// Set while a paragraph draws its glyphs as Chromium's masks
    /// (`glyphs`): whether they are smoothed (`-webkit-font-smoothing:
    /// auto`).
    static PLACE_GLYPHS: Cell<Option<bool>> = const { Cell::new(None) };
}

/// `positions` (user space) moved so that, in device space, x lands on
/// Skia's quarter pixel and y on a whole pixel. `None` when the user space
/// is rotated or skewed (glyphs keep their exact positions).
fn quarter_positions(positions: &[CGPoint], device: &CGAffineTransform) -> Option<Vec<CGPoint>> {
    if device.b != 0.0 || device.c != 0.0 || device.a == 0.0 || device.d == 0.0 {
        return None;
    }
    Some(
        positions
            .iter()
            .map(|point| {
                let x = device.a * point.x + device.tx;
                let y = device.d * point.y + device.ty;
                let qx = ((x + 0.125) * 4.0).floor() / 4.0;
                let qy = (y + 0.5).floor();
                CGPoint::new(point.x + (qx - x) / device.a, point.y + (qy - y) / device.d)
            })
            .collect(),
    )
}

fn word_at(text: &NSString, index: NSUInteger) -> NSRange {
    let length = text.length();
    let mut end = index;
    while end < length {
        let unit = text.characterAtIndex(end);
        if char::from_u32(u32::from(unit)).is_some_and(char::is_whitespace) {
            break;
        }
        end += 1;
        if matches!(unit, 0x2d | 0x2f | 0x2010 | 0x2013 | 0x2014) {
            break;
        }
    }
    NSRange::new(index, end - index)
}

struct Typesetter {
    storage: Retained<NSTextStorage>,
    manager: Retained<NSLayoutManager>,
    lines: Retained<LineHeights>,
    container: Retained<WordContainer>,
    fonts: RefCell<HashMap<FontKey, Retained<NSFont>>>,
    /// Paragraph and width the storage currently holds.
    loaded: Cell<(u64, u32)>,
    next_serial: Cell<u64>,
    /// Paragraphs measured lately, by what decides their size: the layout
    /// engine measures the same text at several widths.
    measured: RefCell<VecDeque<(u64, Paragraph)>>,
}

thread_local! {
    static TYPESETTER: Typesetter = Typesetter::new();
}

impl Typesetter {
    fn new() -> Self {
        let storage = NSTextStorage::new();
        let manager = Retained::into_super(GlyphPlacer::new());
        let container = WordContainer::new();
        // Text starts exactly at the origin the view gives.
        container.setLineFragmentPadding(0.0);
        manager.addTextContainer(&container);
        // Paragraphs are laid out on demand, never behind our back.
        manager.setBackgroundLayoutEnabled(false);
        manager.setUsesFontLeading(true);
        storage.addLayoutManager(&manager);
        let lines = LineHeights::new();
        manager.setDelegate(Some(ProtocolObject::from_ref(&*lines)));
        Self {
            storage,
            manager,
            lines,
            container,
            fonts: RefCell::new(HashMap::new()),
            loaded: Cell::new((0, 0)),
            next_serial: Cell::new(1),
            measured: RefCell::new(VecDeque::new()),
        }
    }

    fn font(&self, spec: &FontSpec) -> Retained<NSFont> {
        let key = FontKey {
            family: spec.family.clone(),
            size: spec.size.to_bits(),
            weight: spec.weight,
            italic: spec.italic,
            features: spec.features.clone(),
        };
        if let Some(font) = self.fonts.borrow().get(&key) {
            return font.clone();
        }
        let font = resolve_font(spec);
        let mut fonts = self.fonts.borrow_mut();
        if fonts.len() >= FONT_CACHE_LIMIT {
            fonts.clear();
        }
        fonts.insert(key, font.clone());
        font
    }

    /// Lays `paragraph` out in a column of `width` (infinite = one line per
    /// paragraph break).
    fn load(&self, paragraph: &Paragraph, width: f32) {
        let key = (paragraph.serial, width.to_bits());
        if self.loaded.get() == key {
            return;
        }
        let column = if width.is_finite() {
            CGFloat::from(width.max(0.0))
        } else {
            UNBOUNDED
        };
        self.container.setSize(CGSize::new(column, UNBOUNDED));
        self.container
            .setMaximumNumberOfLines(paragraph.max_lines as usize);
        // The container's mode decides how its last line is cut.
        self.container
            .setLineBreakMode(if paragraph.max_lines == 0 {
                NSLineBreakMode::ByWordWrapping
            } else {
                match paragraph.ellipsize {
                    Ellipsize::Tail => NSLineBreakMode::ByTruncatingTail,
                    Ellipsize::Head => NSLineBreakMode::ByTruncatingHead,
                    Ellipsize::Middle => NSLineBreakMode::ByTruncatingMiddle,
                    Ellipsize::Clip => NSLineBreakMode::ByClipping,
                }
            });
        self.container.setLineBreakMode(
            if paragraph.wrap == TextWrap::Char && paragraph.max_lines == 0 {
                NSLineBreakMode::ByCharWrapping
            } else {
                self.container.lineBreakMode()
            },
        );
        self.container.ivars().set(false);
        self.lines.ivars().replace(paragraph.line_rule.clone());
        self.storage.setAttributedString(&paragraph.string);
        self.loaded.set(key);
        if paragraph.wrap == TextWrap::Word && width.is_finite() && self.breaks_inside_a_word() {
            self.container.ivars().set(true);
            let all = NSRange::new(0, self.storage.length());
            // SAFETY: the range covers the loaded string; no actual range is asked for.
            unsafe {
                self.manager
                    .invalidateLayoutForCharacterRange_actualCharacterRange(
                        all,
                        std::ptr::null_mut(),
                    );
            }
        }
    }

    /// Whether TextKit had to break a line inside a word: neither side of
    /// some line end is a space, a hyphen or a slash.
    fn breaks_inside_a_word(&self) -> bool {
        let glyphs = self.glyphs();
        let text = self.storage.string();
        let end = glyphs.location + glyphs.length;
        let mut index = glyphs.location;
        while index < end {
            let mut line = NSRange::new(index, 0);
            // SAFETY: `index` is a valid glyph index and `line` outlives the call.
            unsafe {
                self.manager
                    .lineFragmentRectForGlyphAtIndex_effectiveRange(index, &mut line)
            };
            let next = line.location + line.length;
            if next <= index || next >= end {
                break;
            }
            // SAFETY: no actual range is asked for.
            let characters = unsafe {
                self.manager
                    .characterRangeForGlyphRange_actualGlyphRange(line, std::ptr::null_mut())
            };
            let cut = characters.location + characters.length;
            if cut > 0 && cut < text.length() {
                let breakable = |unit: u16| {
                    char::from_u32(u32::from(unit)).is_some_and(char::is_whitespace)
                        || matches!(unit, 0x2d | 0x2f | 0x2010 | 0x2013 | 0x2014)
                };
                if !breakable(text.characterAtIndex(cut - 1))
                    && !breakable(text.characterAtIndex(cut))
                {
                    return true;
                }
            }
            index = next;
        }
        false
    }

    /// The glyphs of the loaded paragraph, laid out.
    fn glyphs(&self) -> NSRange {
        self.manager.glyphRangeForTextContainer(&self.container)
    }

    fn metrics(&self, paragraph: &Paragraph, max_width: f32) -> TextMetrics {
        let glyphs = self.glyphs();
        let used = self.manager.usedRectForTextContainer(&self.container);
        let end = glyphs.location + glyphs.length;
        let mut index = glyphs.location;
        let mut line_count = 0_u32;
        let mut first_baseline = 0.0_f32;
        while index < end {
            let mut line = NSRange::new(index, 0);
            // SAFETY: `index` is a valid glyph index and `line` outlives the call.
            let rect = unsafe {
                self.manager
                    .lineFragmentRectForGlyphAtIndex_effectiveRange(index, &mut line)
            };
            if line_count == 0 {
                first_baseline =
                    (rect.origin.y + self.manager.locationForGlyphAtIndex(index).y) as f32;
            }
            line_count += 1;
            let next = line.location + line.length;
            if next <= index {
                break;
            }
            index = next;
        }
        // A trailing line break opens one more, empty, line.
        if self.manager.extraLineFragmentRect().size.height > 0.0 {
            line_count += 1;
        }
        // Round the width up: a box narrower than its text by a fraction
        // would wrap when drawn (TextKit wraps at the exact advance width); a word that overflows the column does not
        // widen the box (CSS). The height stays exact so fractional line
        // heights stack like a browser's line boxes instead of drifting a
        // pixel per paragraph.
        TextMetrics {
            width: if paragraph.empty {
                0.0
            } else {
                // Up to a Blink LayoutUnit (1/64 px): a whole pixel makes
                // every shrink-wrapped label up to 1px wider than Chromium.
                (((used.size.width * 64.0).ceil() / 64.0) as f32).min(max_width)
            },
            height: used.size.height as f32,
            first_baseline,
            line_count: line_count.max(1),
        }
    }
}

/// AppKit's weight scale (-1..1) for a CSS weight.
fn system_weight(weight: u16) -> CGFloat {
    const STOPS: [(f64, f64); 9] = [
        (100.0, -0.8),
        (200.0, -0.6),
        (300.0, -0.4),
        (400.0, 0.0),
        (500.0, 0.23),
        (600.0, 0.3),
        (700.0, 0.4),
        (800.0, 0.56),
        (900.0, 0.62),
    ];
    let weight = f64::from(weight).clamp(100.0, 900.0);
    for pair in STOPS.windows(2) {
        let ((low, from), (high, to)) = (pair[0], pair[1]);
        if weight <= high {
            return from + (to - from) * (weight - low) / (high - low);
        }
    }
    0.62
}

/// The font manager's 0..15 weight scale for a CSS weight.
fn manager_weight(weight: u16) -> isize {
    match weight {
        0..=149 => 2,
        150..=249 => 3,
        250..=349 => 4,
        350..=449 => 5,
        450..=549 => 6,
        550..=649 => 8,
        650..=749 => 9,
        750..=849 => 10,
        _ => 12,
    }
}

/// `"tnum", "liga" 0` as the font descriptor's feature settings.
fn feature_settings(
    features: &str,
) -> Option<Retained<NSArray<NSDictionary<NSString, AnyObject>>>> {
    let mut settings: Vec<Retained<NSDictionary<NSString, AnyObject>>> = Vec::new();
    for entry in features.split(',') {
        let mut parts = entry.split_whitespace();
        let Some(tag) = parts.next().map(|tag| tag.trim_matches(['"', '\''])) else {
            continue;
        };
        if tag.len() != 4 || !tag.is_ascii() {
            continue;
        }
        let value = match parts.next() {
            None | Some("on") => 1,
            Some("off") => 0,
            Some(number) => number.parse::<i32>().unwrap_or(1),
        };
        // SAFETY: both constants are immutable strings exported by Core Text.
        let (tag_key, value_key): (&NSString, &NSString) = unsafe {
            (
                kCTFontOpenTypeFeatureTag.as_ref(),
                kCTFontOpenTypeFeatureValue.as_ref(),
            )
        };
        let tag = ns_string(tag);
        let value = NSNumber::new_i32(value);
        settings.push(NSDictionary::from_slices(
            &[tag_key, value_key],
            &[any(&*tag), any(&*value)],
        ));
    }
    (!settings.is_empty()).then(|| NSArray::from_retained_slice(&settings))
}

/// The system fonts are variable: Chromium sets their `wght` axis to the
/// CSS weight itself (900 is wght 900), while AppKit's weight traits pick
/// named instances (Black is wght 1000, a wider face).
fn css_weight(font: Retained<NSFont>, weight: u16, size: CGFloat) -> Retained<NSFont> {
    const WGHT: i32 = i32::from_be_bytes(*b"wght");
    let axis = NSNumber::new_i32(WGHT);
    let value = NSNumber::new_f64(f64::from(weight.clamp(1, 1000)));
    let variation: Retained<NSDictionary<NSNumber, NSNumber>> =
        NSDictionary::from_slices(&[&*axis], &[&*value]);
    // SAFETY: the attribute name is an immutable string exported by AppKit.
    let key: &NSString = unsafe { NSFontVariationAttribute };
    let attributes: Retained<NSDictionary<NSString, AnyObject>> =
        NSDictionary::from_slices(&[key], &[any(&*variation)]);
    // SAFETY: the variation attribute takes a dictionary of axis tags to
    // values, as documented.
    let descriptor = unsafe {
        font.fontDescriptor()
            .fontDescriptorByAddingAttributes(&attributes)
    };
    NSFont::fontWithDescriptor_size(&descriptor, size).unwrap_or(font)
}

/// The size of the font that shapes and draws `size` px text. Chromium lays
/// text out and rasterizes it at the CSS size truncated to 1/64 px (as on
/// Linux): 0.78rem (12.48px) text is 12.46875px text, about 0.1% narrower,
/// enough to move a long line's glyphs by a quarter pixel. Only with
/// Chromium's glyph masks: Core Graphics' own quantization rounds the
/// difference away less often than it shifts it.
fn face_size(size: f32) -> CGFloat {
    let size = CGFloat::from(size.max(1.0));
    if crate::glyphs::enabled() {
        (size * 64.0).floor() / 64.0
    } else {
        size
    }
}

fn resolve_font(spec: &FontSpec) -> Retained<NSFont> {
    let size = face_size(spec.size);
    let weight = system_weight(spec.weight);
    let system = || {
        css_weight(
            NSFont::systemFontOfSize_weight(size, weight),
            spec.weight,
            size,
        )
    };
    let designed = |design: &NSString| {
        system()
            .fontDescriptor()
            .fontDescriptorWithDesign(design)
            .and_then(|descriptor| NSFont::fontWithDescriptor_size(&descriptor, size))
    };
    // A CSS family list: the first family that exists wins.
    let mut font = None;
    let mut generic_italic = spec.italic;
    for family in pam_desktop_ui::engine::font_family::families(&spec.family) {
        use pam_desktop_ui::engine::font_family::Family;
        let candidate = match family {
            Family::Asset(source) => crate::fonts::font(source, size),
            Family::SystemUi => Some(system()),
            Family::Monospace => Some(css_weight(
                NSFont::monospacedSystemFontOfSize_weight(size, weight),
                spec.weight,
                size,
            )),
            // SAFETY: the design names are immutable strings exported by AppKit.
            Family::Serif => designed(unsafe { NSFontDescriptorSystemDesignSerif }),
            // SAFETY: as above.
            Family::Rounded => designed(unsafe { NSFontDescriptorSystemDesignRounded }),
            Family::Named(family) => MainThreadMarker::new().and_then(|mtm| {
                let manager = NSFontManager::sharedFontManager(mtm);
                let (family, weight) = (ns_string(family), manager_weight(spec.weight));
                let face = |traits: NSFontTraitMask| {
                    manager.fontWithFamily_traits_weight_size(&family, traits, weight, size)
                };
                match spec
                    .italic
                    .then(|| face(NSFontTraitMask::ItalicFontMask))
                    .flatten()
                {
                    Some(italic) => {
                        // The family has an italic face of its own.
                        generic_italic = false;
                        Some(italic)
                    }
                    None => face(NSFontTraitMask::empty()),
                }
            }),
        };
        if candidate.is_some() {
            font = candidate;
            break;
        }
    }
    let mut font = font.unwrap_or_else(system);
    if generic_italic {
        let descriptor = font.fontDescriptor();
        let traits = descriptor.symbolicTraits() | NSFontDescriptorSymbolicTraits::TraitItalic;
        if let Some(italic) = NSFont::fontWithDescriptor_size(
            &descriptor.fontDescriptorWithSymbolicTraits(traits),
            size,
        ) {
            font = italic;
        }
    }
    if let Some(settings) = feature_settings(&spec.features) {
        // SAFETY: the attribute name is an immutable string exported by AppKit.
        let key: &NSString = unsafe { NSFontFeatureSettingsAttribute };
        let attributes: Retained<NSDictionary<NSString, AnyObject>> =
            NSDictionary::from_slices(&[key], &[any(&*settings)]);
        // SAFETY: the value of the feature settings attribute is an array of
        // dictionaries with the OpenType tag and value keys, as documented.
        let descriptor: Retained<NSFontDescriptor> = unsafe {
            font.fontDescriptor()
                .fontDescriptorByAddingAttributes(&attributes)
        };
        if let Some(featured) = NSFont::fontWithDescriptor_size(&descriptor, size) {
            font = featured;
        }
    }
    font
}

/// The platform font of a font description (cached).
pub fn font(spec: &FontSpec) -> Retained<NSFont> {
    TYPESETTER.with(|typesetter| typesetter.font(spec))
}

/// Blink's `line-height: normal` for `font`.
fn normal_line_height(font: &NSFont) -> f32 {
    (font.ascender().round() + (-font.descender()).round() + font.leading().max(0.0).round()) as f32
}

/// Height of one line of `font` as TextKit lays it out.
pub fn line_height(font: &NSFont) -> f32 {
    TYPESETTER.with(|typesetter| typesetter.manager.defaultLineHeightForFont(font) as f32)
}

pub fn alignment(align: TextAlign) -> NSTextAlignment {
    match align {
        TextAlign::Start => NSTextAlignment::Left,
        TextAlign::Center => NSTextAlignment::Center,
        TextAlign::End => NSTextAlignment::Right,
        TextAlign::Justify => NSTextAlignment::Justified,
    }
}

impl Paragraph {
    pub fn new(block: &TextBlock) -> Self {
        TYPESETTER.with(|typesetter| {
            let style = NSMutableParagraphStyle::new();
            style.setAlignment(alignment(block.align));
            style.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
            let fonts: Vec<Retained<NSFont>> = block
                .runs
                .iter()
                .map(|run| typesetter.font(&run.font))
                .collect();
            let mut baseline_shift = None;
            let mut tall_lines = None;
            // `line-height: normal` is Blink's line spacing: ascent, descent
            // and line gap each rounded to whole pixels (TextKit adds them
            // unrounded and rounds up, a pixel taller with San Francisco).
            let line_height = if block.line_height > 0.0 {
                block.line_height
            } else {
                fonts
                    .iter()
                    .map(|font| normal_line_height(font))
                    .fold(0.0, f32::max)
            };
            if line_height > 0.0 {
                // An inline-block (a `<kbd>` chip) makes its line as tall as
                // its line height plus vertical padding and border. TextKit
                // styles whole paragraphs, so the paragraph takes the tallest.
                let height = CGFloat::from(
                    block
                        .boxes
                        .iter()
                        .filter(|inline| inline.inline_block)
                        .map(InlineBox::line_box_height)
                        .fold(line_height, f32::max),
                );
                style.setMinimumLineHeight(height);
                style.setMaximumLineHeight(height);
                if height > CGFloat::from(line_height) + 0.001 {
                    tall_lines = Some(CGFloat::from(line_height));
                }
                // TextKit puts the extra height above the glyphs; CSS splits
                // it evenly above and below.
                let natural = fonts
                    .iter()
                    .map(|font| typesetter.manager.defaultLineHeightForFont(font))
                    .fold(0.0, CGFloat::max);
                if natural > 0.0 && height > natural {
                    baseline_shift = Some(NSNumber::new_f64((height - natural) / 2.0));
                }
            }

            let empty = block.is_empty();
            let string = NSMutableAttributedString::new();
            let mut length = 0;
            let default_run;
            let (runs, fonts): (&[pam_desktop_ui::text::TextRun], Vec<Retained<NSFont>>) = if empty
            {
                let font = block
                    .runs
                    .first()
                    .map(|run| run.font.clone())
                    .unwrap_or_default();
                default_run = [pam_desktop_ui::text::TextRun {
                    text: String::from(" "),
                    font: font.clone(),
                    color: None,
                    background: None,
                    underline: false,
                    strikethrough: false,
                    letter_spacing: 0.0,
                    press: None,
                }];
                (&default_run, vec![typesetter.font(&font)])
            } else {
                (&block.runs, fonts)
            };
            // Inline box edges are characters of the box text's font
            // (so the line never grows), never drawn, whose advance is the
            // edge's padding plus border: the line breaker sees them.
            let spacers = if empty { Vec::new() } else { block.spacers() };
            let mut next_spacer = 0;
            let mut layout_length = 0_usize;
            let mut box_starts = vec![0_usize; block.boxes.len()];
            let mut box_ranges: Vec<(NSRange, InlineBox, Retained<NSFont>)> = Vec::new();
            let shift = baseline_shift.as_ref().map_or(0.0, |shift| shift.as_f64());
            let mut insert_spacers =
                |string: &NSMutableAttributedString, run: usize, layout_length: &mut usize| {
                    while let Some(spacer) =
                        spacers.get(next_spacer).filter(|spacer| spacer.run == run)
                    {
                        next_spacer += 1;
                        let inline = &block.boxes[spacer.box_index];
                        let Some(font) = fonts.get(inline.runs.start) else {
                            continue;
                        };
                        let character = if spacer.character == SPACER_OPEN {
                            box_starts[spacer.box_index] = *layout_length;
                            SPACER_OPEN
                        } else {
                            let start = box_starts[spacer.box_index];
                            box_ranges.push((
                                NSRange::new(start, *layout_length + 1 - start),
                                inline.clone(),
                                font.clone(),
                            ));
                            LAYOUT_SPACER_CLOSE
                        };
                        let kern = CGFloat::from(spacer.width) - advance(font, character);
                        let attributes: Retained<
                            NSMutableDictionary<NSAttributedStringKey, AnyObject>,
                        > = NSMutableDictionary::new();
                        // SAFETY: as for the runs below.
                        unsafe {
                            attributes.insert(NSFontAttributeName, any(&**font));
                            attributes.insert(NSParagraphStyleAttributeName, any(&*style));
                            attributes.insert(
                                NSForegroundColorAttributeName,
                                any(&*NSColor::clearColor()),
                            );
                            attributes.insert(NSKernAttributeName, any(&*NSNumber::new_f64(kern)));
                            if shift != 0.0 {
                                attributes.insert(
                                    NSBaselineOffsetAttributeName,
                                    any(&*NSNumber::new_f64(shift)),
                                );
                            }
                            let piece = NSAttributedString::initWithString_attributes(
                                NSAttributedString::alloc(),
                                &ns_string(&character.to_string()),
                                Some(&attributes),
                            );
                            string.appendAttributedString(&piece);
                        }
                        *layout_length += 1;
                    }
                };
            for (index, (run, font)) in runs.iter().zip(&fonts).enumerate() {
                insert_spacers(&string, index, &mut layout_length);
                if run.text.is_empty() {
                    continue;
                }
                layout_length += utf16_len(&run.text);
                if !empty {
                    length += utf16_len(&run.text);
                }
                let attributes: Retained<NSMutableDictionary<NSAttributedStringKey, AnyObject>> =
                    NSMutableDictionary::new();
                // SAFETY: the attribute names are immutable strings exported
                // by AppKit, and every value has the class its attribute documents.
                unsafe {
                    attributes.insert(NSFontAttributeName, any(&**font));
                    attributes.insert(NSParagraphStyleAttributeName, any(&*style));
                    let color = run.color.map_or_else(NSColor::labelColor, ns_color);
                    attributes.insert(NSForegroundColorAttributeName, any(&*color));
                    if let Some(background) = run.background {
                        attributes
                            .insert(NSBackgroundColorAttributeName, any(&*ns_color(background)));
                    }
                    if run.underline {
                        attributes
                            .insert(NSUnderlineStyleAttributeName, any(&*NSNumber::new_isize(1)));
                    }
                    if run.strikethrough {
                        attributes.insert(
                            NSStrikethroughStyleAttributeName,
                            any(&*NSNumber::new_isize(1)),
                        );
                    }
                    if run.letter_spacing != 0.0 {
                        attributes.insert(
                            NSKernAttributeName,
                            any(&*NSNumber::new_f64(f64::from(run.letter_spacing))),
                        );
                    }
                    if let Some(shift) = &baseline_shift {
                        attributes.insert(NSBaselineOffsetAttributeName, any(&**shift));
                    }
                    let piece = NSAttributedString::initWithString_attributes(
                        NSAttributedString::alloc(),
                        &ns_string(&run.text),
                        Some(&attributes),
                    );
                    string.appendAttributedString(&piece);
                }
            }
            insert_spacers(&string, runs.len(), &mut layout_length);
            let chromium_baseline: f32 = runs
                .iter()
                .zip(&fonts)
                .enumerate()
                .filter(|(_, (run, _))| !run.text.is_empty())
                .map(|(index, (_, font))| {
                    // A run inside a box with its own line height takes
                    // that line height; an inline-block also holds its
                    // top padding and border.
                    let (height, edge) = block
                        .boxes
                        .iter()
                        .rev()
                        .find(|inline| inline.runs.contains(&index) && inline.line_height > 0.0)
                        .map_or((block.line_height, 0.0), |inline| {
                            (inline.line_height, inline.edge_above())
                        });
                    let ascent = font.ascender().round() as f32;
                    let descent = (-font.descender()).round() as f32;
                    let height = if height > 0.0 {
                        height
                    } else {
                        normal_line_height(font)
                    };
                    let leading = if height > 0.0 {
                        ((height - ascent - descent) / 2.0).floor()
                    } else {
                        0.0
                    };
                    edge + ascent + leading
                })
                .fold(0.0, f32::max);
            let serial = typesetter.next_serial.get();
            typesetter.next_serial.set(serial + 1);
            Self {
                string: Retained::into_super(string),
                serial,
                max_lines: block.max_lines,
                ellipsize: block.ellipsize,
                wrap: block.wrap,
                antialiased: block.antialiased,
                line_phase: Cell::new((0.0, 1.0)),
                chromium_baseline,
                shadow: block.shadow,
                length,
                empty,
                map: if empty {
                    SpacerMap::default()
                } else {
                    block.spacer_map()
                },
                layout_length,
                line_rule: tall_lines.map(|base| {
                    // Chromium's baseline of a line without a box: the
                    // block's strut (ascent, descent, floored half-leading).
                    let plain = fonts
                        .iter()
                        .map(|font| {
                            let ascent = font.ascender().round();
                            let descent = (-font.descender()).round();
                            ascent + ((base - ascent - descent) / 2.0).floor()
                        })
                        .fold(0.0, CGFloat::max);
                    LineRule {
                        base,
                        baseline: plain - CGFloat::from(chromium_baseline),
                        boxes: box_ranges
                            .iter()
                            .filter(|(_, inline, _)| inline.inline_block)
                            .map(|(range, _, _)| *range)
                            .collect(),
                    }
                }),
                boxes: box_ranges,
            }
        })
    }

    /// Paints the inline boxes under the text: one fragment per line a box
    /// spans, as tall as its font's content area (or its own line height)
    /// plus vertical padding and border, like CSS. The line never grows.
    fn draw_boxes(&self, typesetter: &Typesetter, context: &CGContext, origin: CGPoint) {
        let manager = &typesetter.manager;
        for (range, inline, font) in &self.boxes {
            // SAFETY: the range lies inside the loaded string; no actual range is asked for.
            let glyphs = unsafe {
                manager
                    .glyphRangeForCharacterRange_actualCharacterRange(*range, std::ptr::null_mut())
            };
            let end = glyphs.location + glyphs.length;
            let mut index = glyphs.location;
            let (ascent, descent) = (font.ascender(), -font.descender());
            let (above, below) = if inline.line_height > 0.0 {
                let leading = (CGFloat::from(inline.line_height) - ascent - descent) / 2.0;
                (ascent + leading, descent + leading)
            } else {
                (ascent, descent)
            };
            while index < end {
                let mut line = NSRange::new(index, 0);
                // SAFETY: `index` is a laid-out glyph and `line` outlives the call.
                let fragment = unsafe {
                    manager.lineFragmentRectForGlyphAtIndex_effectiveRange(index, &mut line)
                };
                let line_end = (line.location + line.length).min(end);
                if line.length == 0 || line_end <= index {
                    break;
                }
                let piece = NSRange::new(index, line_end - index);
                let bounds =
                    manager.boundingRectForGlyphRange_inTextContainer(piece, &typesetter.container);
                // The glyph location already includes the baseline offset.
                let baseline = fragment.origin.y + manager.locationForGlyphAtIndex(index).y;
                let top = baseline - above - CGFloat::from(inline.padding[0] + inline.border[0]);
                let bottom = baseline + below + CGFloat::from(inline.padding[2] + inline.border[2]);
                let rect = CGRect::new(
                    CGPoint::new(origin.x + bounds.origin.x, origin.y + top),
                    CGSize::new(bounds.size.width, bottom - top),
                );
                paint_box(context, rect, inline);
                index = line_end;
            }
        }
    }

    /// How far the paragraph must move down so its first baseline sits
    /// where Chromium puts it (TextKit splits fractional leading evenly and
    /// keeps fractional ascents).
    pub fn baseline_shift(&self, width: f32) -> f32 {
        if self.chromium_baseline <= 0.0 {
            return 0.0;
        }
        self.chromium_baseline - self.measure(width).first_baseline
    }

    /// The size the glyphs take when wrapped to `width`, without capping the
    /// width at the column: an overlong word overflows it (CSS
    /// `overflow: visible`).
    pub fn ink_size(&self, width: f32) -> (f32, f32) {
        TYPESETTER.with(|typesetter| {
            typesetter.load(self, width);
            let used = typesetter
                .manager
                .usedRectForTextContainer(&typesetter.container);
            (used.size.width as f32, used.size.height as f32)
        })
    }

    /// The metrics the layout engine receives: the first baseline where
    /// Chromium puts it, as the view draws it (baselines align boxes).
    fn reported(&self, width: f32) -> TextMetrics {
        let metrics = self.measure(width);
        if self.chromium_baseline > 0.0 {
            TextMetrics {
                first_baseline: self.chromium_baseline,
                ..metrics
            }
        } else {
            metrics
        }
    }

    /// Size of the paragraph wrapped to `width`.
    pub fn measure(&self, width: f32) -> TextMetrics {
        TYPESETTER.with(|typesetter| {
            typesetter.load(self, width);
            typesetter.metrics(self, width)
        })
    }

    /// Where the next `draw` starts: `phase` is the paragraph's exact top
    /// minus the snapped one, in device pixels at `scale`.
    pub fn set_line_phase(&self, phase: f32, scale: f32) {
        self.line_phase
            .set((f64::from(phase), f64::from(scale.max(1.0))));
    }

    /// Each line's glyphs and drawing origin. Blink snaps every line box's
    /// top on its own (round half up of the exact top), so lines a
    /// fractional line height apart land 18 or 19 pixels apart, and each
    /// baseline on a whole pixel; TextKit stacks them at exact fractions.
    fn snapped_lines(
        &self,
        typesetter: &Typesetter,
        glyphs: NSRange,
        origin: CGPoint,
    ) -> Vec<(NSRange, CGPoint)> {
        let (phase, scale) = self.line_phase.get();
        let end = glyphs.location + glyphs.length;
        let mut lines = Vec::new();
        let mut index = glyphs.location;
        while index < end {
            let mut line = NSRange::new(index, 0);
            // SAFETY: `index` is a valid glyph index and `line` outlives the call.
            let rect = unsafe {
                typesetter
                    .manager
                    .lineFragmentRectForGlyphAtIndex_effectiveRange(index, &mut line)
            };
            let top = rect.origin.y * scale;
            let shift = ((phase + top + 0.5).floor() - top) / scale;
            let next = (line.location + line.length).min(end);
            if next <= index {
                lines.push((
                    NSRange::new(index, end - index),
                    CGPoint::new(origin.x, origin.y + shift),
                ));
                break;
            }
            lines.push((
                NSRange::new(index, next - index),
                CGPoint::new(origin.x, origin.y + shift),
            ));
            index = next;
        }
        lines
    }

    /// Paints the paragraph with its top-left corner at `origin`, in the
    /// current (flipped) graphics context. `selection` is highlighted
    /// behind its glyphs.
    pub fn draw(
        &self,
        context: &CGContext,
        origin: CGPoint,
        width: f32,
        selection: Option<(u32, u32)>,
    ) {
        self.draw_glyphs(context, origin, width, selection, true);
    }

    /// Paints the paragraph into a printed page: glyphs keep their exact
    /// positions (a page has no pixel grid).
    pub fn draw_printed(&self, context: &CGContext, origin: CGPoint, width: f32) {
        self.draw_glyphs(context, origin, width, None, false);
    }

    fn draw_glyphs(
        &self,
        context: &CGContext,
        origin: CGPoint,
        width: f32,
        selection: Option<(u32, u32)>,
        place: bool,
    ) {
        TYPESETTER.with(|typesetter| {
            typesetter.load(self, width);
            let glyphs = typesetter.glyphs();
            if glyphs.length == 0 {
                return;
            }
            let selected = selection
                .map(|(start, end)| {
                    let layout = |offset: u32, after: bool| {
                        (self.map.layout_index(offset.min(self.length as u32), after) as usize)
                            .min(self.layout_length)
                    };
                    layout(start, true)..layout(end, false)
                })
                .filter(|range| !self.empty && range.start < range.end)
                .map(|range| NSRange::new(range.start, range.end - range.start));
            if let Some(range) = selected {
                let color = NSColor::selectedTextBackgroundColor();
                // SAFETY: the background attribute takes a color; the range
                // lies inside the text. Temporary attributes never re-layout.
                unsafe {
                    typesetter
                        .manager
                        .addTemporaryAttribute_value_forCharacterRange(
                            NSBackgroundColorAttributeName,
                            any(&*color),
                            range,
                        )
                };
            }
            self.draw_boxes(typesetter, context, origin);
            // Chromium's glyph masks (`glyphs`, opt-in): TextKit's runs go to
            // `GlyphPlacer`, which draws each glyph on Skia's quarter pixel.
            // Not under a shadow, which Core Graphics would cast per glyph.
            let masks = place && self.shadow.is_none() && crate::glyphs::enabled();
            // Otherwise Core Graphics draws the glyphs at their exact
            // advances in quarter pixels (without subpixel positioning it
            // rounds each glyph to a whole pixel and words drift by up to a
            // pixel). Skia rounds each glyph to the nearest quarter; Core
            // Graphics truncates: an eighth of a pixel turns one into the
            // other.
            let glyph_origin = if masks {
                origin
            } else {
                let (_, scale) = self.line_phase.get();
                CGPoint::new(origin.x + 0.125 / scale, origin.y)
            };
            let lines = self.snapped_lines(typesetter, glyphs, glyph_origin);
            for &(range, at) in &lines {
                typesetter
                    .manager
                    .drawBackgroundForGlyphRange_atPoint(range, at);
            }
            CGContext::save_g_state(Some(context));
            CGContext::set_allows_font_subpixel_positioning(Some(context), true);
            CGContext::set_should_subpixel_position_fonts(Some(context), true);
            CGContext::set_allows_font_subpixel_quantization(Some(context), !masks);
            CGContext::set_should_subpixel_quantize_fonts(Some(context), !masks);
            let placing = PLACE_GLYPHS.replace(masks.then_some(!self.antialiased));
            if self.antialiased {
                // `-webkit-font-smoothing: antialiased`: no stem darkening.
                CGContext::save_g_state(Some(context));
                CGContext::set_should_smooth_fonts(Some(context), false);
            }
            if let Some(shadow) = self.shadow {
                CGContext::save_g_state(Some(context));
                let offset = crate::paint::shadow_offset(shadow.offset);
                CGContext::set_shadow_with_color(
                    Some(context),
                    offset,
                    CGFloat::from(shadow.radius.max(0.0)),
                    Some(&cg_color(shadow.color)),
                );
                for &(range, at) in &lines {
                    typesetter
                        .manager
                        .drawGlyphsForGlyphRange_atPoint(range, at);
                }
                CGContext::restore_g_state(Some(context));
            } else {
                for &(range, at) in &lines {
                    typesetter
                        .manager
                        .drawGlyphsForGlyphRange_atPoint(range, at);
                }
            }
            if self.antialiased {
                CGContext::restore_g_state(Some(context));
            }
            PLACE_GLYPHS.set(placing);
            CGContext::restore_g_state(Some(context));
            if let Some(range) = selected {
                // SAFETY: the attribute name is an immutable string exported by AppKit.
                typesetter
                    .manager
                    .removeTemporaryAttribute_forCharacterRange(
                        unsafe { NSBackgroundColorAttributeName },
                        range,
                    );
            }
        });
    }

    /// The UTF-16 offset of the character boundary nearest to `point`,
    /// given relative to the paragraph's top-left corner.
    pub fn index_at(&self, width: f32, point: CGPoint) -> u32 {
        if self.empty {
            return 0;
        }
        TYPESETTER.with(|typesetter| {
            typesetter.load(self, width);
            if typesetter.glyphs().length == 0 {
                return 0;
            }
            let mut fraction: CGFloat = 0.0;
            // SAFETY: `fraction` outlives the call.
            let index = unsafe {
                typesetter.manager.characterIndexForPoint_inTextContainer_fractionOfDistanceBetweenInsertionPoints(
                    point,
                    &typesetter.container,
                    &mut fraction,
                )
            };
            let mut offset = index.min(self.layout_length);
            if fraction > 0.5 && offset < self.layout_length {
                // Past the middle of a character: the boundary after it,
                // never inside a surrogate pair or a composed sequence.
                let character = typesetter.storage.string().rangeOfComposedCharacterSequenceAtIndex(offset);
                offset = character.location + character.length;
            }
            self.map
                .paragraph_index(offset.min(self.layout_length) as u32)
                .min(self.length as u32)
        })
    }
}

/// Measures a paragraph exactly as a view will draw it.
pub fn measure(block: &TextBlock, max_width: f32) -> TextMetrics {
    // Everything that decides the size, whatever the width.
    let key = block.layout_key(0.0);
    let cached = TYPESETTER.with(|typesetter| {
        typesetter
            .measured
            .borrow()
            .iter()
            .find(|(cached, _)| *cached == key)
            .map(|(_, paragraph)| paragraph.reported(max_width))
    });
    if let Some(metrics) = cached {
        return metrics;
    }
    let paragraph = Paragraph::new(block);
    let metrics = paragraph.reported(max_width);
    TYPESETTER.with(|typesetter| {
        let mut measured = typesetter.measured.borrow_mut();
        if measured.len() >= MEASURE_CACHE_LIMIT {
            measured.pop_front();
        }
        measured.push_back((key, paragraph));
    });
    metrics
}

/// One fragment of an inline box: background under the padding and border,
/// then the border, with the box's corner radius.
fn paint_box(context: &CGContext, rect: CGRect, inline: &InlineBox) {
    if rect.size.width <= 0.0 || rect.size.height <= 0.0 {
        return;
    }
    let color = inline.border_color.unwrap_or_default();
    let [top, right, bottom, left] = inline.border;
    let decoration = Decoration {
        background: inline.background.unwrap_or_default(),
        radii: Radii::all(inline.radius),
        border: Border {
            widths: [left, top, right, bottom],
            colors: [color; 4],
            ..Border::default()
        },
        ..Decoration::default()
    };
    let size = Size::new(rect.size.width as f32, rect.size.height as f32);
    CGContext::save_g_state(Some(context));
    CGContext::translate_ctm(Some(context), rect.origin.x, rect.origin.y);
    crate::paint::paint_background(context, size, &decoration);
    crate::paint::paint_border(context, size, &decoration);
    CGContext::restore_g_state(Some(context));
}

#[cfg(test)]
mod tests {
    use super::*;
    use pam_desktop_ui::text::TextRun;

    fn run(text: &str) -> TextRun {
        TextRun {
            text: text.to_owned(),
            font: FontSpec {
                family: "Helvetica".into(),
                size: 14.0,
                ..FontSpec::default()
            },
            letter_spacing: 0.0,
            color: None,
            background: None,
            underline: false,
            strikethrough: false,
            press: None,
        }
    }

    /// `a <kbd>Ctrl</kbd> b`: the box's horizontal padding and border push
    /// the neighbours, keep the line height, and are not characters of the
    /// paragraph for hit testing.
    #[test]
    fn inline_boxes_take_their_edges_in_the_line() {
        let plain = TextBlock {
            runs: vec![run("a "), run("Ctrl"), run(" b")],
            ..TextBlock::default()
        };
        let boxed = TextBlock {
            boxes: vec![InlineBox {
                runs: 1..2,
                padding: [1.0, 6.0, 1.0, 6.0],
                border: [1.0, 1.0, 2.0, 1.0],
                border_color: Some(pam_desktop_ui::color::Color::WHITE),
                radius: 4.0,
                background: Some(pam_desktop_ui::color::Color::BLACK),
                line_height: 0.0,
                inline_block: false,
            }],
            ..plain.clone()
        };
        let without = Paragraph::new(&plain).measure(f32::INFINITY);
        let paragraph = Paragraph::new(&boxed);
        let with = paragraph.measure(f32::INFINITY);
        assert!(
            (with.width - without.width - 14.0).abs() <= 1.0,
            "{} -> {}",
            without.width,
            with.width
        );
        assert_eq!(with.height, without.height);
        assert_eq!(with.line_count, 1);
        // Far right is the end of the paragraph (8 characters), not of the
        // laid-out string (10 with the two spacers).
        assert_eq!(
            paragraph.index_at(f32::INFINITY, CGPoint::new(1000.0, 5.0)),
            8
        );
        assert_eq!(paragraph.index_at(f32::INFINITY, CGPoint::new(0.0, 5.0)), 0);
    }

    /// CSS `overflow-wrap: normal`: a word wider than the column overflows
    /// it on a line of its own instead of being cut between characters;
    /// `break-all` (`TextWrap::Char`) still cuts it.
    #[test]
    fn words_wider_than_the_column_are_not_broken() {
        let block = TextBlock {
            runs: vec![run("Disponíveis para este")],
            ..TextBlock::default()
        };
        let word = Paragraph::new(&TextBlock {
            runs: vec![run("Disponíveis")],
            ..TextBlock::default()
        })
        .measure(f32::INFINITY);
        let narrow = word.width / 2.0;
        let metrics = Paragraph::new(&block).measure(narrow);
        assert_eq!(metrics.line_count, 3);
        assert!(metrics.width <= narrow);
        let char_wrapped = Paragraph::new(&TextBlock {
            wrap: TextWrap::Char,
            ..block
        })
        .measure(narrow);
        assert!(char_wrapped.line_count > 3);
    }

    /// Baselines sit where Chromium puts them: whole pixels, ascent rounded
    /// and `floor(half-leading)` above, whatever the fractional line height.
    #[test]
    fn baselines_follow_chromium() {
        for line_height in [0.0, 13.5, 17.024, 18.4, 24.0] {
            let block = TextBlock {
                runs: vec![run("Hxg")],
                line_height,
                ..TextBlock::default()
            };
            let paragraph = Paragraph::new(&block);
            let font = font(&block.runs[0].font);
            let (ascent, descent) = (
                font.ascender().round() as f32,
                (-font.descender()).round() as f32,
            );
            let expected = ascent
                + if line_height > 0.0 {
                    ((line_height - ascent - descent) / 2.0).floor()
                } else {
                    0.0
                };
            let baseline = paragraph.measure(f32::INFINITY).first_baseline
                + paragraph.baseline_shift(f32::INFINITY);
            assert!(
                (baseline - expected).abs() < 0.001,
                "{line_height}: {baseline} != {expected}"
            );
            assert_eq!(baseline.fract(), 0.0);
        }
    }

    /// Lines 18.5625 px apart are drawn from the pixel of each exact top
    /// (round half up), like Blink: 0, 19, 37 from a whole-pixel top and
    /// 0, 18, 37 from a top 0.4 px above its snapped pixel.
    #[test]
    fn lines_snap_like_blink() {
        let block = TextBlock {
            runs: vec![run("one\ntwo\nthree")],
            line_height: 18.5625,
            ..TextBlock::default()
        };
        let paragraph = Paragraph::new(&block);
        for (phase, expected) in [(0.0, [0.0, 19.0, 37.0]), (-0.4, [0.0, 18.0, 37.0])] {
            paragraph.set_line_phase(phase, 1.0);
            TYPESETTER.with(|typesetter| {
                typesetter.load(&paragraph, f32::INFINITY);
                let lines = paragraph.snapped_lines(typesetter, typesetter.glyphs(), CGPoint::ZERO);
                let tops: Vec<f64> = lines
                    .iter()
                    .map(|&(range, at)| {
                        // SAFETY: the range starts at a laid-out glyph.
                        let rect = unsafe {
                            typesetter
                                .manager
                                .lineFragmentRectForGlyphAtIndex_effectiveRange(
                                    range.location,
                                    std::ptr::null_mut(),
                                )
                        };
                        rect.origin.y + at.y
                    })
                    .collect();
                assert_eq!(tops, expected, "phase {phase}");
            });
        }
    }

    /// An inline-block taller than the line grows its own line only: the
    /// next line keeps the block's line height, like CSS line boxes.
    #[test]
    fn inline_blocks_grow_only_their_line() {
        let block = TextBlock {
            runs: vec![run("a "), run("Ctrl"), run(" b\nlast")],
            boxes: vec![InlineBox {
                runs: 1..2,
                padding: [2.0, 6.0, 2.0, 6.0],
                border: [1.0, 1.0, 1.0, 1.0],
                border_color: None,
                radius: 4.0,
                background: None,
                line_height: 20.0,
                inline_block: true,
            }],
            line_height: 20.0,
            ..TextBlock::default()
        };
        let metrics = Paragraph::new(&block).measure(f32::INFINITY);
        assert_eq!(metrics.line_count, 2);
        assert_eq!(metrics.height, 26.0 + 20.0);
    }
}
