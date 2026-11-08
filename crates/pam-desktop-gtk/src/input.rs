//! Native text entry: `GtkText` for single-line fields and `GtkTextView` for
//! editors. The surrounding box (background, border, radius) is painted by
//! the `PamView` that contains the control, so a field looks exactly like the
//! application styled it while typing, selection, input methods and
//! accessibility stay native.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

use gtk::prelude::*;
use gtk::{gdk, glib};
use pam_desktop_ui::backend::{
    BackendEvent, Capitalization, InputConfig, InputEvent, InputPurpose, NodeId, TextContent,
    VerticalAlign,
};
use pam_desktop_ui::color::Color;
use pam_desktop_ui::geometry::{Rect, Size};
use pam_desktop_ui::runtime::EventQueue;
use pam_desktop_ui::text::{
    TextAlign, TextBlock, TextRun, char_to_utf16_offset, utf16_to_char_offset,
};

use crate::view::{self, PamView};

static NEXT_CLASS: AtomicU64 = AtomicU64::new(1);

enum Control {
    Line(gtk::Text),
    Area {
        scrolled: gtk::ScrolledWindow,
        view: gtk::TextView,
    },
}

pub struct InputHost {
    id: NodeId,
    control: Control,
    /// Set while the host itself changes the text or the selection.
    updating: Rc<Cell<bool>>,
    config: Rc<RefCell<InputConfig>>,
    provider: gtk::CssProvider,
    class: String,
    size: Size,
}

fn css_color(color: Color) -> String {
    format!(
        "rgba({},{},{},{:.3})",
        color.r,
        color.g,
        color.b,
        f32::from(color.a) / 255.0
    )
}

/// Reports an input event once GTK finished the signal emission it came
/// from: the core may answer by replacing the text (masks), which GTK does
/// not allow in the middle of a user edit.
fn emit(events: &Rc<EventQueue>, event: BackendEvent) {
    let events = Rc::clone(events);
    glib::idle_add_local_full(glib::Priority::HIGH, move || {
        events.push(event.clone());
        glib::ControlFlow::Break
    });
}

fn key_text(key: gdk::Key, state: gdk::ModifierType) -> Option<String> {
    if state.intersects(
        gdk::ModifierType::CONTROL_MASK
            | gdk::ModifierType::ALT_MASK
            | gdk::ModifierType::META_MASK,
    ) {
        return None;
    }
    match key {
        gdk::Key::BackSpace => Some("Backspace".into()),
        gdk::Key::Return | gdk::Key::KP_Enter | gdk::Key::ISO_Enter => Some("Enter".into()),
        _ => key
            .to_unicode()
            .filter(|character| !character.is_control())
            .map(|character| character.to_string()),
    }
}

impl InputHost {
    pub fn new(id: NodeId, multiline: bool, container: &PamView, events: &Rc<EventQueue>) -> Self {
        let updating = Rc::new(Cell::new(false));
        let config = Rc::new(RefCell::new(InputConfig::default()));
        let class = format!("pam-input-{}", NEXT_CLASS.fetch_add(1, Ordering::Relaxed));
        let provider = gtk::CssProvider::new();
        if let Some(display) = gdk::Display::default() {
            gtk::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
            );
        }

        let control = if multiline {
            let view = gtk::TextView::new();
            view.add_css_class(&class);
            view.set_wrap_mode(gtk::WrapMode::WordChar);
            view.set_accepts_tab(false);
            let scrolled = gtk::ScrolledWindow::new();
            scrolled.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
            scrolled.set_propagate_natural_height(false);
            scrolled.set_child(Some(&view));
            scrolled.set_parent(container);

            let buffer = view.buffer();
            {
                let events = Rc::clone(events);
                let updating = Rc::clone(&updating);
                let container = container.downgrade();
                let config = Rc::clone(&config);
                buffer.connect_changed(move |buffer| {
                    let text = buffer
                        .text(&buffer.start_iter(), &buffer.end_iter(), false)
                        .to_string();
                    if let Some(container) = container.upgrade() {
                        show_placeholder(&container, &config.borrow(), text.is_empty());
                    }
                    if updating.get() {
                        return;
                    }
                    let caret = char_to_utf16_offset(&text, buffer.cursor_position().max(0) as u32);
                    emit(
                        &events,
                        BackendEvent::Input {
                            id,
                            event: InputEvent::Edited {
                                text,
                                selection: (caret, caret),
                            },
                        },
                    );
                });
            }
            {
                let events = Rc::clone(events);
                let updating = Rc::clone(&updating);
                buffer.connect_cursor_position_notify(move |buffer| {
                    if updating.get() {
                        return;
                    }
                    let text = buffer
                        .text(&buffer.start_iter(), &buffer.end_iter(), false)
                        .to_string();
                    let (start, end) = match buffer.selection_bounds() {
                        Some((start, end)) => (start.offset(), end.offset()),
                        None => (buffer.cursor_position(), buffer.cursor_position()),
                    };
                    emit(
                        &events,
                        BackendEvent::Input {
                            id,
                            event: InputEvent::SelectionChanged {
                                start: char_to_utf16_offset(&text, start.max(0) as u32),
                                end: char_to_utf16_offset(&text, end.max(0) as u32),
                            },
                        },
                    );
                });
            }
            {
                let keys = gtk::EventControllerKey::new();
                keys.set_propagation_phase(gtk::PropagationPhase::Capture);
                let events = Rc::clone(events);
                let config = Rc::clone(&config);
                keys.connect_key_pressed(move |_, key, _, state| {
                    let Some(text) = key_text(key, state) else {
                        return glib::Propagation::Proceed;
                    };
                    let submits = text == "Enter"
                        && config.borrow().submit_on_enter
                        && !state.contains(gdk::ModifierType::SHIFT_MASK);
                    emit(
                        &events,
                        BackendEvent::Input {
                            id,
                            event: InputEvent::KeyPressed { key: text },
                        },
                    );
                    if submits {
                        emit(
                            &events,
                            BackendEvent::Input {
                                id,
                                event: InputEvent::Submitted,
                            },
                        );
                        return glib::Propagation::Stop;
                    }
                    glib::Propagation::Proceed
                });
                view.add_controller(keys);
            }
            Control::Area { scrolled, view }
        } else {
            let text = gtk::Text::new();
            text.add_css_class(&class);
            text.set_propagate_text_width(false);
            text.set_truncate_multiline(true);
            text.set_width_chars(0);
            text.set_max_width_chars(0);
            text.set_parent(container);
            {
                let events = Rc::clone(events);
                let updating = Rc::clone(&updating);
                text.connect_changed(move |text| {
                    if updating.get() {
                        return;
                    }
                    let value = text.text().to_string();
                    let caret = char_to_utf16_offset(&value, text.position().max(0) as u32);
                    emit(
                        &events,
                        BackendEvent::Input {
                            id,
                            event: InputEvent::Edited {
                                text: value,
                                selection: (caret, caret),
                            },
                        },
                    );
                });
            }
            {
                let events = Rc::clone(events);
                text.connect_activate(move |_| {
                    emit(
                        &events,
                        BackendEvent::Input {
                            id,
                            event: InputEvent::Submitted,
                        },
                    )
                });
            }
            for property in ["cursor-position", "selection-bound"] {
                let events = Rc::clone(events);
                let updating = Rc::clone(&updating);
                text.connect_notify_local(Some(property), move |text, _| {
                    if updating.get() {
                        return;
                    }
                    let value = text.text().to_string();
                    let (start, end) = text
                        .selection_bounds()
                        .unwrap_or((text.position(), text.position()));
                    emit(
                        &events,
                        BackendEvent::Input {
                            id,
                            event: InputEvent::SelectionChanged {
                                start: char_to_utf16_offset(&value, start.max(0) as u32),
                                end: char_to_utf16_offset(&value, end.max(0) as u32),
                            },
                        },
                    );
                });
            }
            {
                let keys = gtk::EventControllerKey::new();
                keys.set_propagation_phase(gtk::PropagationPhase::Capture);
                let events = Rc::clone(events);
                keys.connect_key_pressed(move |_, key, _, state| {
                    if let Some(key) = key_text(key, state) {
                        emit(
                            &events,
                            BackendEvent::Input {
                                id,
                                event: InputEvent::KeyPressed { key },
                            },
                        );
                    }
                    glib::Propagation::Proceed
                });
                text.add_controller(keys);
            }
            Control::Line(text)
        };

        // Select everything on focus when the application asks for it.
        {
            let focus = gtk::EventControllerFocus::new();
            let config = Rc::clone(&config);
            let target = match &control {
                Control::Line(text) => text.clone().upcast::<gtk::Widget>(),
                Control::Area { view, .. } => view.clone().upcast::<gtk::Widget>(),
            };
            let weak = target.downgrade();
            focus.connect_enter(move |_| {
                if !config.borrow().select_on_focus {
                    return;
                }
                let Some(widget) = weak.upgrade() else { return };
                // After GTK placed the caret for the click that focused it.
                glib::idle_add_local_once(move || {
                    if let Some(text) = widget.downcast_ref::<gtk::Text>() {
                        text.select_region(0, -1);
                    } else if let Some(view) = widget.downcast_ref::<gtk::TextView>() {
                        let buffer = view.buffer();
                        buffer.select_range(&buffer.start_iter(), &buffer.end_iter());
                    }
                });
            });
            target.add_controller(focus);
        }

        Self {
            id,
            control,
            updating,
            config,
            provider,
            class,
            size: Size::ZERO,
        }
    }

    /// The widget placed inside the container.
    fn widget(&self) -> gtk::Widget {
        match &self.control {
            Control::Line(text) => text.clone().upcast(),
            Control::Area { scrolled, .. } => scrolled.clone().upcast(),
        }
    }

    /// The widget that takes keyboard focus.
    pub fn focus_target(&self) -> gtk::Widget {
        match &self.control {
            Control::Line(text) => text.clone().upcast(),
            Control::Area { view, .. } => view.clone().upcast(),
        }
    }

    fn current_text(&self) -> String {
        match &self.control {
            Control::Line(text) => text.text().to_string(),
            Control::Area { view, .. } => {
                let buffer = view.buffer();
                buffer
                    .text(&buffer.start_iter(), &buffer.end_iter(), false)
                    .to_string()
            }
        }
    }

    /// Places the control inside the box, within padding and borders.
    pub fn layout(&mut self, size: Size, container: &PamView) {
        self.size = size;
        let insets = self.config.borrow().insets;
        let frame = Rect::new(
            insets.left,
            insets.top,
            (size.width - insets.horizontal()).max(0.0),
            (size.height - insets.vertical()).max(0.0),
        );
        view::set_layout(&self.widget(), |layout| layout.frame = frame);
        container.queue_allocate();
    }

    pub fn configure(&mut self, config: &InputConfig, container: &PamView) {
        *self.config.borrow_mut() = config.clone();

        let mut rules = format!(
            "font-size: {}px; font-weight: {}; font-style: {}; letter-spacing: {}px; \
             background: none; border: none; box-shadow: none; outline: none; padding: 0; margin: 0; min-height: 0;",
            config.font.size,
            config.font.weight,
            if config.font.italic {
                "italic"
            } else {
                "normal"
            },
            config.letter_spacing,
        );
        if !config.font.family.is_empty() {
            rules.push_str(&format!(
                " font-family: \"{}\";",
                crate::fonts::family(&config.font.family)
                    .0
                    .replace(['"', '\\', ';', '{', '}'], "")
            ));
        }
        if let Some(color) = config.color {
            rules.push_str(&format!(" color: {};", css_color(color)));
        }
        if config.caret_hidden {
            rules.push_str(" caret-color: transparent;");
        } else if let Some(color) = config.caret_color {
            rules.push_str(&format!(" caret-color: {};", css_color(color)));
        }
        let class = &self.class;
        let mut css = format!(".{class}, .{class} text {{ {rules} }}\n");
        match config.placeholder_color {
            Some(color) => css.push_str(&format!(
                ".{class} > placeholder {{ color: {}; opacity: 1; }}\n",
                css_color(color)
            )),
            // Without an authored colour the hint is a dimmed version of the text.
            None => css.push_str(&format!(
                ".{class} > placeholder {{ color: alpha(currentColor, 0.45); opacity: 1; }}\n"
            )),
        }
        if let Some(color) = config.selection_color {
            css.push_str(&format!(
                ".{class} selection, .{class} text selection {{ background-color: {}; }}\n",
                css_color(color)
            ));
        }
        self.provider.load_from_string(&css);

        let purpose = match config.purpose {
            InputPurpose::Text | InputPurpose::Search => gtk::InputPurpose::FreeForm,
            InputPurpose::Email => gtk::InputPurpose::Email,
            InputPurpose::Number => gtk::InputPurpose::Digits,
            InputPurpose::Decimal => gtk::InputPurpose::Number,
            InputPurpose::Phone => gtk::InputPurpose::Phone,
            InputPurpose::Url => gtk::InputPurpose::Url,
        };
        let mut hints = if config.spellcheck {
            gtk::InputHints::SPELLCHECK
        } else {
            gtk::InputHints::NO_SPELLCHECK
        };
        hints |= match config.capitalization {
            Capitalization::None => gtk::InputHints::LOWERCASE,
            Capitalization::Sentences => gtk::InputHints::UPPERCASE_SENTENCES,
            Capitalization::Words => gtk::InputHints::UPPERCASE_WORDS,
            Capitalization::Characters => gtk::InputHints::UPPERCASE_CHARS,
        };
        let alignment = match config.align {
            TextAlign::Start | TextAlign::Justify => 0.0,
            TextAlign::Center => 0.5,
            TextAlign::End => 1.0,
        };
        match &self.control {
            Control::Line(text) => {
                text.set_placeholder_text(
                    (!config.placeholder.is_empty()).then_some(config.placeholder.as_str()),
                );
                // Chromium clips a long placeholder at the field edge; GTK's
                // placeholder label ellipsizes by default.
                let mut child = text.first_child();
                while let Some(current) = child {
                    if let Some(label) = current.downcast_ref::<gtk::Label>() {
                        label.set_ellipsize(gtk::pango::EllipsizeMode::None);
                    }
                    child = current.next_sibling();
                }
                // Use the standard password bullet rather than the theme-dependent
                // oversized black circle; the actual text remains in GtkText secure mode.
                text.set_invisible_char('\u{2022}');
                text.set_visibility(!config.secure);
                text.set_input_purpose(if config.secure {
                    gtk::InputPurpose::Password
                } else {
                    purpose
                });
                text.set_input_hints(hints);
                text.set_editable(config.editable);
                text.set_sensitive(config.enabled);
                text.set_alignment(alignment);
                // GTK counts characters; the protocol counts UTF-16 units. The
                // difference only matters for text outside the basic plane.
                text.set_max_length(
                    config
                        .max_length
                        .map_or(0, |length| i32::try_from(length).unwrap_or(i32::MAX)),
                );
                text.set_valign(match config.vertical {
                    VerticalAlign::Top => gtk::Align::Start,
                    VerticalAlign::Center => gtk::Align::Fill,
                    VerticalAlign::Bottom => gtk::Align::End,
                });
            }
            Control::Area { view, .. } => {
                view.set_input_purpose(purpose);
                view.set_input_hints(hints);
                view.set_editable(config.editable);
                view.set_sensitive(config.enabled);
                view.set_cursor_visible(!config.caret_hidden);
                // A CSS line box: the half-leading of `line-height` sits above
                // and below each line, so the first line starts where a
                // browser textarea starts it.
                let leading = config.line_height.map_or(0, |line_height| {
                    let mut description = gtk::pango::FontDescription::new();
                    if !config.font.family.is_empty() {
                        description.set_family(&config.font.family);
                    }
                    description.set_absolute_size(
                        f64::from(config.font.size) * f64::from(gtk::pango::SCALE),
                    );
                    let metrics = view.pango_context().metrics(Some(&description), None);
                    let natural =
                        (metrics.ascent() + metrics.descent()) as f32 / gtk::pango::SCALE as f32;
                    ((line_height - natural) / 2.0).max(0.0).round() as i32
                });
                view.set_pixels_above_lines(leading);
                view.set_pixels_below_lines(leading);
                view.set_justification(match config.align {
                    TextAlign::Start => gtk::Justification::Left,
                    TextAlign::Center => gtk::Justification::Center,
                    TextAlign::End => gtk::Justification::Right,
                    TextAlign::Justify => gtk::Justification::Fill,
                });
                show_placeholder(container, config, self.current_text().is_empty());
            }
        }
        self.layout(self.size, container);
    }

    pub fn set_text(&self, value: &str, selection: Option<(u32, u32)>) {
        self.updating.set(true);
        match &self.control {
            Control::Line(text) => {
                if text.text() != value {
                    text.set_text(value);
                }
                match selection {
                    Some((start, end)) if start != end => {
                        text.select_region(
                            utf16_to_char_offset(value, start) as i32,
                            utf16_to_char_offset(value, end) as i32,
                        );
                    }
                    Some((start, _)) => {
                        text.set_position(utf16_to_char_offset(value, start) as i32)
                    }
                    None => text.set_position(-1),
                }
            }
            Control::Area { view, .. } => {
                let buffer = view.buffer();
                if buffer.text(&buffer.start_iter(), &buffer.end_iter(), false) != value {
                    buffer.set_text(value);
                }
                let (start, end) = selection.unwrap_or_else(|| {
                    let end = u32::try_from(value.chars().count()).unwrap_or(u32::MAX);
                    (end, end)
                });
                let start = buffer.iter_at_offset(utf16_to_char_offset(value, start) as i32);
                let end = buffer.iter_at_offset(utf16_to_char_offset(value, end) as i32);
                buffer.select_range(&start, &end);
            }
        }
        self.updating.set(false);
    }

    pub fn set_selection(&self, start: u32, end: u32) {
        let value = self.current_text();
        self.updating.set(true);
        let start = utf16_to_char_offset(&value, start) as i32;
        let end = utf16_to_char_offset(&value, end) as i32;
        match &self.control {
            Control::Line(text) => text.select_region(start, end),
            Control::Area { view, .. } => {
                let buffer = view.buffer();
                buffer.select_range(&buffer.iter_at_offset(start), &buffer.iter_at_offset(end));
            }
        }
        self.updating.set(false);
    }

    /// Detaches the control and its stylesheet.
    pub fn destroy(&self) {
        let widget = self.widget();
        if let Some(window) = widget.root().and_downcast::<gtk::Window>()
            && let Some(focused) = gtk::prelude::GtkWindowExt::focus(&window)
            && (focused == widget || focused.is_ancestor(&widget))
        {
            // Deliver focus-out while the GtkText is still attached. Removing
            // a focused modal otherwise leaves GTK's native text focus stale.
            gtk::prelude::GtkWindowExt::set_focus(&window, None::<&gtk::Widget>);
        }
        view::forget_layout(&widget);
        if widget.parent().is_some() {
            widget.unparent();
        }
        if let Some(display) = gdk::Display::default() {
            gtk::style_context_remove_provider_for_display(&display, &self.provider);
        }
        let _ = self.id;
    }
}

/// `GtkTextView` has no placeholder: the box paints it while the editor is empty.
fn show_placeholder(container: &PamView, config: &InputConfig, empty: bool) {
    if !empty || config.placeholder.is_empty() {
        container.set_text(None);
        return;
    }
    // Same fallback as the single-line entry and AppKit: the text colour at 45%.
    let color = config.placeholder_color.unwrap_or_else(|| {
        config
            .color
            .unwrap_or(Color::rgba(128, 128, 128, 255))
            .with_alpha(0.45)
    });
    container.set_text(Some(&TextContent {
        block: TextBlock {
            runs: vec![TextRun {
                text: config.placeholder.clone(),
                font: config.font.clone(),
                color: Some(color),
                background: None,
                underline: false,
                strikethrough: false,
                letter_spacing: config.letter_spacing,
                press: None,
            }],
            align: config.align,
            ..TextBlock::default()
        },
        insets: config.insets,
        vertical: VerticalAlign::Top,
    }));
}

/// Placeholder fallback check, run from the GTK test thread.
#[cfg(test)]
pub(crate) fn assert_placeholder_fallback() {
    let container = PamView::new();
    let config = InputConfig {
        placeholder: "Buscar".to_owned(),
        color: Some(Color::rgba(255, 255, 255, 255)),
        ..InputConfig::default()
    };
    show_placeholder(&container, &config, true);
    let color = container.with_state(|state| {
        state
            .text
            .as_ref()
            .and_then(|text| text.content.block.runs[0].color)
    });
    assert_eq!(color, Some(Color::rgba(255, 255, 255, 115)));
    let authored = InputConfig {
        placeholder_color: Some(Color::rgba(98, 113, 132, 255)),
        ..config
    };
    show_placeholder(&container, &authored, true);
    let color = container.with_state(|state| {
        state
            .text
            .as_ref()
            .and_then(|text| text.content.block.runs[0].color)
    });
    assert_eq!(color, Some(Color::rgba(98, 113, 132, 255)));
}
