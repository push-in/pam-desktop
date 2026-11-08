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
use objc2::{AnyThread, MainThreadMarker};
use objc2_app_kit::{
    NSBackgroundColorAttributeName, NSBaselineOffsetAttributeName, NSColor, NSFont,
    NSFontAttributeName, NSFontDescriptor, NSFontDescriptorSymbolicTraits,
    NSFontDescriptorSystemDesignRounded, NSFontDescriptorSystemDesignSerif,
    NSFontFeatureSettingsAttribute, NSFontManager, NSFontTraitMask, NSForegroundColorAttributeName,
    NSKernAttributeName, NSLayoutManager, NSLineBreakMode, NSMutableParagraphStyle,
    NSParagraphStyleAttributeName, NSStrikethroughStyleAttributeName, NSTextAlignment,
    NSTextContainer, NSTextStorage, NSUnderlineStyleAttributeName,
};
use objc2_core_foundation::{CGFloat, CGPoint, CGRect, CGSize};
use objc2_core_graphics::CGContext;
use objc2_core_text::{
    CTFont, CTFontOrientation, kCTFontOpenTypeFeatureTag, kCTFontOpenTypeFeatureValue,
};
use objc2_foundation::{
    NSArray, NSAttributedString, NSAttributedStringKey, NSDictionary, NSMutableAttributedString,
    NSMutableDictionary, NSNumber, NSRange, NSString,
};
use pam_desktop_ui::geometry::Size;
use pam_desktop_ui::style::{Border, Decoration, Radii};
use pam_desktop_ui::text::{
    Ellipsize, FontSpec, InlineBox, SPACER_OPEN, SpacerMap, TextAlign, TextBlock, TextMetrics,
    TextShadow,
};

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

#[derive(Clone, Eq, Hash, PartialEq)]
struct FontKey {
    family: String,
    size: u32,
    weight: u16,
    italic: bool,
    features: String,
}

struct Typesetter {
    storage: Retained<NSTextStorage>,
    manager: Retained<NSLayoutManager>,
    container: Retained<NSTextContainer>,
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
        let manager = NSLayoutManager::new();
        let container = NSTextContainer::initWithSize(
            NSTextContainer::alloc(),
            CGSize::new(UNBOUNDED, UNBOUNDED),
        );
        // Text starts exactly at the origin the view gives.
        container.setLineFragmentPadding(0.0);
        manager.addTextContainer(&container);
        // Paragraphs are laid out on demand, never behind our back.
        manager.setBackgroundLayoutEnabled(false);
        manager.setUsesFontLeading(true);
        storage.addLayoutManager(&manager);
        Self {
            storage,
            manager,
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
        self.storage.setAttributedString(&paragraph.string);
        self.loaded.set(key);
    }

    /// The glyphs of the loaded paragraph, laid out.
    fn glyphs(&self) -> NSRange {
        self.manager.glyphRangeForTextContainer(&self.container)
    }

    fn metrics(&self, paragraph: &Paragraph) -> TextMetrics {
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
        // Round up: a box narrower than its text by a fraction would wrap when drawn.
        TextMetrics {
            width: if paragraph.empty {
                0.0
            } else {
                used.size.width.ceil() as f32
            },
            height: used.size.height.ceil() as f32,
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

fn resolve_font(spec: &FontSpec) -> Retained<NSFont> {
    let size = CGFloat::from(spec.size.max(1.0));
    let weight = system_weight(spec.weight);
    let system = || NSFont::systemFontOfSize_weight(size, weight);
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
            Family::Monospace => Some(NSFont::monospacedSystemFontOfSize_weight(size, weight)),
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
            if block.line_height > 0.0 {
                let height = CGFloat::from(block.line_height);
                style.setMinimumLineHeight(height);
                style.setMaximumLineHeight(height);
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
            let serial = typesetter.next_serial.get();
            typesetter.next_serial.set(serial + 1);
            Self {
                string: Retained::into_super(string),
                serial,
                max_lines: block.max_lines,
                ellipsize: block.ellipsize,
                shadow: block.shadow,
                length,
                empty,
                map: if empty {
                    SpacerMap::default()
                } else {
                    block.spacer_map()
                },
                layout_length,
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

    /// Size of the paragraph wrapped to `width`.
    pub fn measure(&self, width: f32) -> TextMetrics {
        TYPESETTER.with(|typesetter| {
            typesetter.load(self, width);
            typesetter.metrics(self)
        })
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
            typesetter
                .manager
                .drawBackgroundForGlyphRange_atPoint(glyphs, origin);
            if let Some(shadow) = self.shadow {
                CGContext::save_g_state(Some(context));
                let offset = crate::paint::shadow_offset(shadow.offset);
                CGContext::set_shadow_with_color(
                    Some(context),
                    offset,
                    CGFloat::from(shadow.radius.max(0.0)),
                    Some(&cg_color(shadow.color)),
                );
                typesetter
                    .manager
                    .drawGlyphsForGlyphRange_atPoint(glyphs, origin);
                CGContext::restore_g_state(Some(context));
            } else {
                typesetter
                    .manager
                    .drawGlyphsForGlyphRange_atPoint(glyphs, origin);
            }
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
            .map(|(_, paragraph)| paragraph.measure(max_width))
    });
    if let Some(metrics) = cached {
        return metrics;
    }
    let paragraph = Paragraph::new(block);
    let metrics = paragraph.measure(max_width);
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
}
