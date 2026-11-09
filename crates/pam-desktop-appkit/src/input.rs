//! Native text entry: `NSTextField` (or `NSSecureTextField`) for
//! single-line fields and `NSTextView` in a scroll view for editors. The
//! surrounding box (background, border, radius) is painted by the node's
//! view, so a field looks exactly like the application styled it while
//! typing, selection, input methods and accessibility stay native.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use objc2::rc::{Retained, Weak};
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject, Sel};
use objc2::{
    AnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel,
};
use objc2_app_kit::{
    NSApplication, NSBackgroundColorAttributeName, NSBorderType, NSColor, NSControl,
    NSControlTextEditingDelegate, NSEventModifierFlags, NSEventType, NSFocusRingType,
    NSFontAttributeName, NSForegroundColorAttributeName, NSKernAttributeName, NSLineBreakMode,
    NSMutableParagraphStyle, NSParagraphStyleAttributeName, NSScrollView, NSScrollerStyle,
    NSSecureTextField, NSText, NSTextDelegate, NSTextField, NSTextFieldDelegate, NSTextInputClient,
    NSTextView, NSTextViewDelegate, NSTextViewDidChangeSelectionNotification, NSView,
};
use objc2_core_foundation::{CGRect, CGSize};
use objc2_foundation::{
    NSAttributedString, NSAttributedStringKey, NSDictionary, NSMutableDictionary, NSNotification,
    NSNotificationCenter, NSNumber, NSRange, NSString,
};
use pam_desktop_ui::backend::{
    BackendEvent, InputConfig, InputEvent, InputPurpose, KeyEvent, MAIN_WINDOW, NodeId,
    TextContent, VerticalAlign,
};
use pam_desktop_ui::color::Color;
use pam_desktop_ui::geometry::Size;
use pam_desktop_ui::text::{TextBlock, TextRun};

use crate::events::{self, Events};
use crate::text;
use crate::util::{cg_rect, ns_color, ns_string};
use crate::view::PamView;
use crate::window::PamWindow;

fn any<T: AsRef<AnyObject> + ?Sized>(value: &T) -> &AnyObject {
    value.as_ref()
}

enum Control {
    Line(Retained<NSTextField>),
    Area {
        scroll: Retained<NSScrollView>,
        view: Retained<NSTextView>,
    },
}

/// What a field changed on the window's shared field editor, to put back
/// when it loses focus.
struct EditorStyle {
    caret: Retained<NSColor>,
    selection: Retained<NSDictionary<NSAttributedStringKey, AnyObject>>,
    spelling: bool,
}

/// State shared by the host, the delegate and the focus listener.
struct Shared {
    id: NodeId,
    events: Rc<Events>,
    config: RefCell<InputConfig>,
    /// Set while the host itself changes the text or the selection.
    updating: Cell<bool>,
    /// The single-line field, to recognise its field editor.
    line: RefCell<Option<Weak<NSTextField>>>,
    container: Weak<PamView>,
    /// Size of the editor's text last reported.
    content_size: Cell<Size>,
    editor_style: RefCell<Option<EditorStyle>>,
    /// The field was swapped (password shown or hidden) while focused: the
    /// next focus keeps the caret instead of `selectOnFocus`, like GTK.
    keep_selection: Cell<bool>,
}

impl Shared {
    /// Input events are handled on the next turn of the run loop: the core
    /// may answer by replacing the text (masks), which AppKit does not
    /// expect in the middle of the edit that is being reported.
    fn emit(&self, event: InputEvent) {
        self.events
            .push_later(BackendEvent::Input { id: self.id, event });
    }

    /// The text of the key press being handled, when it types something.
    fn typed_text(&self, mtm: MainThreadMarker) -> Option<String> {
        let event = NSApplication::sharedApplication(mtm).currentEvent()?;
        if event.r#type() != NSEventType::KeyDown {
            return None;
        }
        if event
            .modifierFlags()
            .intersects(NSEventModifierFlags::Command | NSEventModifierFlags::Control)
        {
            return None;
        }
        let text = event.characters()?.to_string();
        let types = !text.is_empty()
            && !text.chars().any(|character| {
                character.is_control() || ('\u{F700}'..='\u{F8FF}').contains(&character)
            });
        types.then_some(text)
    }

    /// The attributes text has in the control (font, color, spacing, alignment).
    fn text_attributes(
        &self,
        color: Option<Color>,
    ) -> Retained<NSMutableDictionary<NSAttributedStringKey, AnyObject>> {
        let config = self.config.borrow();
        let attributes: Retained<NSMutableDictionary<NSAttributedStringKey, AnyObject>> =
            NSMutableDictionary::new();
        let style = NSMutableParagraphStyle::new();
        style.setAlignment(text::alignment(config.align));
        let color = color
            .or(config.color)
            .map_or_else(NSColor::labelColor, ns_color);
        // SAFETY: the attribute names are immutable strings exported by
        // AppKit, and every value has the class its attribute documents.
        unsafe {
            attributes.insert(NSFontAttributeName, any(&*text::font(&config.font)));
            attributes.insert(NSForegroundColorAttributeName, any(&*color));
            attributes.insert(NSParagraphStyleAttributeName, any(&*style));
            if config.letter_spacing != 0.0 {
                attributes.insert(
                    NSKernAttributeName,
                    any(&*NSNumber::new_f64(f64::from(config.letter_spacing))),
                );
            }
        }
        attributes
    }

    fn selection_of(text: &NSText) -> (u32, u32) {
        let range = text.selectedRange();
        (
            range.location as u32,
            (range.location + range.length) as u32,
        )
    }

    /// The user edited the single-line field.
    fn line_changed(&self, field: &NSTextField) {
        if self.updating.get() {
            return;
        }
        let editor = field.currentEditor();
        let mut value = field.stringValue().to_string();
        let limit = self.config.borrow().max_length;
        let composing = editor
            .as_ref()
            .and_then(|editor| editor.downcast_ref::<NSTextView>())
            .is_some_and(|editor| editor.hasMarkedText());
        if let Some(limit) = limit.filter(|_| !composing) {
            // Maximum length in UTF-16 units, never splitting a character.
            let mut units = 0;
            let kept: String = value
                .chars()
                .take_while(|character| {
                    units += character.len_utf16();
                    units <= limit as usize
                })
                .collect();
            if kept.len() != value.len() {
                let caret = editor
                    .as_ref()
                    .map_or(0, |editor| editor.selectedRange().location)
                    .min(kept.encode_utf16().count());
                self.updating.set(true);
                field.setStringValue(&ns_string(&kept));
                if let Some(editor) = &editor {
                    editor.setSelectedRange(NSRange::new(caret, 0));
                }
                self.updating.set(false);
                value = kept;
            }
        }
        if let Some(key) = self.typed_text(field.mtm()) {
            self.emit(InputEvent::KeyPressed { key });
        }
        let length = value.encode_utf16().count() as u32;
        let selection = editor
            .as_ref()
            .map_or((length, length), |editor| Self::selection_of(editor));
        self.emit(InputEvent::Edited {
            text: value,
            selection,
        });
    }

    fn selection_changed(&self, text: &NSText) {
        if self.updating.get() {
            return;
        }
        let (start, end) = Self::selection_of(text);
        self.emit(InputEvent::SelectionChanged { start, end });
    }

    /// The user edited the multi-line editor.
    fn area_changed(&self, view: &NSTextView) {
        self.refresh_placeholder(view.string().length() == 0);
        self.report_content_size(view);
        if self.updating.get() {
            return;
        }
        self.emit(InputEvent::Edited {
            text: view.string().to_string(),
            selection: Self::selection_of(view),
        });
    }

    /// Size of the laid out text, for editors that grow with their content.
    fn report_content_size(&self, view: &NSTextView) {
        // SAFETY: the layout manager and container belong to the text view
        // and are only used during this call.
        let (Some(manager), Some(container)) = (unsafe { view.layoutManager() }, unsafe {
            view.textContainer()
        }) else {
            return;
        };
        manager.ensureLayoutForTextContainer(&container);
        let used = manager.usedRectForTextContainer(&container).size;
        let size = Size::new(used.width.ceil() as f32, used.height.ceil() as f32);
        if self.content_size.replace(size) != size {
            self.emit(InputEvent::ContentSizeChanged { size });
        }
    }

    /// `NSTextView` has no placeholder: the box paints it while the
    /// editor is empty.
    fn refresh_placeholder(&self, empty: bool) {
        let Some(container) = self.container.load() else {
            return;
        };
        let config = self.config.borrow();
        if !empty || config.placeholder.is_empty() {
            container.set_text(None);
            return;
        }
        let color = placeholder_color(&config).unwrap_or(Color::rgba(128, 128, 128, 200));
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
            inline: false,
        }));
    }

    /// An editing command of the key bindings. Returns whether it was
    /// handled here instead of by the text view.
    fn command(&self, view: &NSTextView, selector: Sel, multiline: bool) -> bool {
        let mtm = view.mtm();
        let shift = NSApplication::sharedApplication(mtm)
            .currentEvent()
            .is_some_and(|event| event.modifierFlags().contains(NSEventModifierFlags::Shift));
        if selector == sel!(insertNewline:)
            || (!multiline && selector == sel!(insertNewlineIgnoringFieldEditor:))
        {
            self.emit(InputEvent::KeyPressed {
                key: String::from("Enter"),
            });
            let submits = !multiline || (self.config.borrow().submit_on_enter && !shift);
            if submits {
                self.emit(InputEvent::Submitted);
            }
            // A field keeps editing after Enter; an editor that does not
            // submit inserts the line break itself.
            return submits;
        }
        if selector == sel!(deleteBackward:) {
            self.emit(InputEvent::KeyPressed {
                key: String::from("Backspace"),
            });
            return false;
        }
        if multiline && (selector == sel!(insertTab:) || selector == sel!(insertBacktab:)) {
            // Tab leaves the editor, as it leaves a field.
            if let Some(window) = view.window() {
                if selector == sel!(insertTab:) {
                    window.selectKeyViewFollowingView(view);
                } else {
                    window.selectKeyViewPrecedingView(view);
                }
            }
            return true;
        }
        if selector == sel!(cancelOperation:) || selector == sel!(noop:) {
            // A key the control has no use for (Escape, an unbound chord)
            // goes to the core, like a key no native control consumed.
            let Some(event) = NSApplication::sharedApplication(mtm)
                .currentEvent()
                .filter(|event| event.r#type() == NSEventType::KeyDown)
            else {
                return false;
            };
            let window = view
                .window()
                .and_then(|window| window.downcast::<PamWindow>().ok())
                .map_or(MAIN_WINDOW, |window| window.id());
            self.events.push_later(BackendEvent::Key(KeyEvent {
                window,
                pressed: true,
                key: events::key_name(&event),
                modifiers: events::modifiers(event.modifierFlags()),
                repeat: event.isARepeat(),
            }));
            return true;
        }
        false
    }

    /// Whether an edit of the editor fits in the maximum length.
    fn allows_change(
        &self,
        view: &NSTextView,
        range: NSRange,
        replacement: Option<&NSString>,
    ) -> bool {
        let Some(replacement) = replacement else {
            return true;
        };
        let inserted = replacement.length();
        let fits = self.config.borrow().max_length.is_none_or(|limit| {
            inserted == 0
                || (view.string().length() + inserted).saturating_sub(range.length)
                    <= limit as usize
        });
        if fits
            && !self.updating.get()
            && inserted > 0
            && replacement.to_string() != "\n"
            && let Some(key) = self.typed_text(view.mtm())
        {
            self.emit(InputEvent::KeyPressed { key });
        }
        fits
    }

    /// Caret, selection color and spelling of an editing text view.
    fn style_editor(&self, editor: &NSTextView) {
        let config = self.config.borrow();
        if config.caret_hidden {
            editor.setInsertionPointColor(Some(&NSColor::clearColor()));
        } else if let Some(color) = config.caret_color {
            editor.setInsertionPointColor(Some(&ns_color(color)));
        }
        if let Some(color) = config.selection_color {
            let color = ns_color(color);
            // SAFETY: the attribute name is an immutable string exported
            // by AppKit and its value is a color.
            unsafe {
                let key: &NSString = NSBackgroundColorAttributeName;
                let attributes: Retained<NSDictionary<NSAttributedStringKey, AnyObject>> =
                    NSDictionary::from_slices(&[key], &[any(&*color)]);
                editor.setSelectedTextAttributes(&attributes);
            }
        }
        let plain = matches!(config.purpose, InputPurpose::Text | InputPurpose::Search);
        editor.setContinuousSpellCheckingEnabled(config.spellcheck && plain && !config.secure);
        if config.letter_spacing != 0.0 {
            drop(config);
            let attributes = self.text_attributes(None);
            // SAFETY: the attributes are the standard text attributes
            // built above; the storage belongs to the editor.
            unsafe {
                editor.setTypingAttributes(&attributes);
                if let Some(storage) = editor.textStorage() {
                    let everything = NSRange::new(0, storage.length());
                    storage.addAttribute_value_range(
                        NSKernAttributeName,
                        any(&*NSNumber::new_f64(f64::from(
                            self.config.borrow().letter_spacing,
                        ))),
                        everything,
                    );
                }
            }
        }
    }

    /// Keyboard focus entered or left the single-line field. Returns
    /// `false` when the field has focus but its editor is not attached yet.
    fn line_focus_changed(&self, focused: bool) -> bool {
        let Some(field) = self.line.borrow().as_ref().and_then(Weak::load) else {
            return true;
        };
        if focused {
            let Some(editor) = field
                .currentEditor()
                .and_then(|editor| editor.downcast::<NSTextView>().ok())
            else {
                return false;
            };
            // The field editor is shared by every field of the window.
            *self.editor_style.borrow_mut() = Some(EditorStyle {
                caret: editor.insertionPointColor(),
                selection: editor.selectedTextAttributes(),
                spelling: editor.isContinuousSpellCheckingEnabled(),
            });
            self.style_editor(&editor);
            let keep = self.keep_selection.replace(false);
            if self.config.borrow().select_on_focus && !keep {
                // After AppKit placed the caret for the click that focused it.
                let field = Weak::from_retained(&field);
                self.events.defer(move || {
                    if let Some(field) =
                        field.load().filter(|field| field.currentEditor().is_some())
                    {
                        // SAFETY: `selectText:` accepts a nil sender.
                        unsafe { field.selectText(None) };
                    }
                });
            }
        } else {
            let style = self.editor_style.borrow_mut().take();
            let (Some(style), Some(window)) = (style, field.window()) else {
                return true;
            };
            // SAFETY: asks for the existing shared field editor only.
            let Some(editor) = (unsafe { window.fieldEditor_forObject(false, None) })
                .and_then(|editor| editor.downcast::<NSTextView>().ok())
            else {
                return true;
            };
            editor.setInsertionPointColor(Some(&style.caret));
            // SAFETY: the attributes were read from this same text view.
            unsafe { editor.setSelectedTextAttributes(&style.selection) };
            editor.setContinuousSpellCheckingEnabled(style.spelling);
            self.restyle_line(&field);
        }
        true
    }

    /// Letter spacing of a field that is not being edited: its cell only
    /// draws it from an attributed value.
    fn restyle_line(&self, field: &NSTextField) {
        if self.config.borrow().letter_spacing == 0.0 || field.currentEditor().is_some() {
            return;
        }
        let attributes = self.text_attributes(None);
        // SAFETY: the attributes are the standard text attributes built above.
        let value = unsafe {
            NSAttributedString::initWithString_attributes(
                NSAttributedString::alloc(),
                &field.stringValue(),
                Some(&attributes),
            )
        };
        self.updating.set(true);
        field.setAttributedStringValue(&value);
        self.updating.set(false);
    }
}

define_class!(
    // SAFETY: `NSObject` has no subclassing requirements. Main thread only.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = Rc<Shared>]
    struct InputDelegate;

    impl InputDelegate {
        /// Some text view of the application changed its selection: ours
        /// when it is the field editor of the single-line field.
        #[unsafe(method(editorSelectionChanged:))]
        fn editor_selection_changed(&self, notification: &NSNotification) {
            let shared = self.ivars();
            let Some(field) = shared.line.borrow().as_ref().and_then(Weak::load) else { return };
            let (Some(editor), Some(source)) = (field.currentEditor(), notification.object()) else { return };
            if Retained::as_ptr(&editor).cast::<AnyObject>() == Retained::as_ptr(&source) {
                shared.selection_changed(&editor);
            }
        }
    }

    unsafe impl NSObjectProtocol for InputDelegate {}

    unsafe impl NSControlTextEditingDelegate for InputDelegate {
        #[unsafe(method(controlTextDidChange:))]
        fn control_text_did_change(&self, notification: &NSNotification) {
            if let Some(field) = notification.object().and_then(|object| object.downcast::<NSTextField>().ok()) {
                self.ivars().line_changed(&field);
            }
        }

        #[unsafe(method(control:textView:doCommandBySelector:))]
        fn control_do_command(&self, _control: &NSControl, view: &NSTextView, selector: Sel) -> bool {
            self.ivars().command(view, selector, false)
        }
    }

    unsafe impl NSTextFieldDelegate for InputDelegate {}

    unsafe impl NSTextDelegate for InputDelegate {
        #[unsafe(method(textDidChange:))]
        fn text_did_change(&self, notification: &NSNotification) {
            if let Some(view) = notification.object().and_then(|object| object.downcast::<NSTextView>().ok()) {
                self.ivars().area_changed(&view);
            }
        }
    }

    unsafe impl NSTextViewDelegate for InputDelegate {
        #[unsafe(method(textViewDidChangeSelection:))]
        fn text_view_did_change_selection(&self, notification: &NSNotification) {
            if let Some(view) = notification.object().and_then(|object| object.downcast::<NSTextView>().ok()) {
                self.ivars().selection_changed(&view);
            }
        }

        #[unsafe(method(textView:doCommandBySelector:))]
        fn text_view_do_command(&self, view: &NSTextView, selector: Sel) -> bool {
            self.ivars().command(view, selector, true)
        }

        #[unsafe(method(textView:shouldChangeTextInRange:replacementString:))]
        fn text_view_should_change(&self, view: &NSTextView, range: NSRange, replacement: Option<&NSString>) -> bool {
            self.ivars().allows_change(view, range, replacement)
        }
    }
);

pub struct InputHost {
    mtm: MainThreadMarker,
    control: Control,
    delegate: Retained<InputDelegate>,
    shared: Rc<Shared>,
    secure: bool,
    size: Size,
}

fn make_line(mtm: MainThreadMarker, secure: bool) -> Retained<NSTextField> {
    let field = if secure {
        Retained::into_super(NSSecureTextField::initWithFrame(
            NSSecureTextField::alloc(mtm),
            CGRect::ZERO,
        ))
    } else {
        NSTextField::initWithFrame(NSTextField::alloc(mtm), CGRect::ZERO)
    };
    // The node's view paints the box; the field is only the text.
    field.setBezeled(false);
    field.setBordered(false);
    field.setDrawsBackground(false);
    field.setFocusRingType(NSFocusRingType::None);
    field.setUsesSingleLineMode(true);
    field.setLineBreakMode(NSLineBreakMode::ByClipping);
    if let Some(cell) = field.cell() {
        cell.setScrollable(true);
        cell.setWraps(false);
    }
    field
}

impl InputHost {
    pub fn new(
        mtm: MainThreadMarker,
        id: NodeId,
        multiline: bool,
        container: &PamView,
        parent: &NSView,
        events: &Rc<Events>,
    ) -> Self {
        let shared = Rc::new(Shared {
            id,
            events: Rc::clone(events),
            config: RefCell::new(InputConfig::default()),
            updating: Cell::new(false),
            line: RefCell::new(None),
            container: Weak::from_retained(&container.retain_view()),
            content_size: Cell::new(Size::ZERO),
            editor_style: RefCell::new(None),
            keep_selection: Cell::new(false),
        });
        let this = InputDelegate::alloc(mtm).set_ivars(Rc::clone(&shared));
        // SAFETY: `init` is the designated initializer of `NSObject`.
        let delegate: Retained<InputDelegate> = unsafe { msg_send![super(this), init] };

        let control = if multiline {
            let scroll = NSTextView::scrollableTextView(mtm);
            let view = scroll
                .documentView()
                .and_then(|document| document.downcast::<NSTextView>().ok())
                .unwrap_or_else(|| {
                    // Never taken: the factory always builds a text view.
                    let view = NSTextView::initWithFrame(NSTextView::alloc(mtm), CGRect::ZERO);
                    scroll.setDocumentView(Some(&view));
                    view
                });
            scroll.setDrawsBackground(false);
            scroll.setBorderType(NSBorderType::NoBorder);
            scroll.setHasVerticalScroller(true);
            scroll.setAutohidesScrollers(true);
            scroll.setScrollerStyle(NSScrollerStyle::Overlay);
            scroll.setAutomaticallyAdjustsContentInsets(false);
            // The application styles the focused state of its fields.
            scroll.setFocusRingType(NSFocusRingType::None);
            view.setFocusRingType(NSFocusRingType::None);
            view.setDrawsBackground(false);
            view.setRichText(false);
            view.setImportsGraphics(false);
            view.setAllowsUndo(true);
            view.setUsesFontPanel(false);
            view.setUsesRuler(false);
            // The text starts exactly at the box's content corner.
            view.setTextContainerInset(CGSize::new(0.0, 0.0));
            // SAFETY: the container belongs to the text view just created.
            if let Some(container) = unsafe { view.textContainer() } {
                container.setLineFragmentPadding(0.0);
            }
            view.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
            parent.addSubview(&scroll);
            Control::Area { scroll, view }
        } else {
            let field = make_line(mtm, false);
            parent.addSubview(&field);
            Control::Line(field)
        };
        let host = Self {
            mtm,
            control,
            delegate,
            shared,
            secure: false,
            size: Size::ZERO,
        };
        host.attach(container);
        host
    }

    /// Connects the current control to the delegate and to focus changes.
    fn attach(&self, container: &PamView) {
        match &self.control {
            Control::Line(field) => {
                *self.shared.line.borrow_mut() = Some(Weak::from_retained(field));
                // SAFETY: the delegate implements the text field delegate
                // protocol and outlives the field (the host owns both).
                unsafe { field.setDelegate(Some(ProtocolObject::from_ref(&*self.delegate))) };
                // SAFETY: `editorSelectionChanged:` is implemented by the
                // delegate; the observer is removed in `destroy`.
                unsafe {
                    let center = NSNotificationCenter::defaultCenter();
                    // Only this registration: `setDelegate:` just registered
                    // the delegate for the control's own notifications
                    // (`controlTextDidChange:`), which a plain
                    // `removeObserver:` would drop.
                    center.removeObserver_name_object(
                        &self.delegate,
                        Some(NSTextViewDidChangeSelectionNotification),
                        None,
                    );
                    center.addObserver_selector_name_object(
                        &self.delegate,
                        sel!(editorSelectionChanged:),
                        Some(NSTextViewDidChangeSelectionNotification),
                        None,
                    );
                }
                let shared = Rc::clone(&self.shared);
                container.set_focus_listener(Some(Rc::new(move |focused| {
                    if !shared.line_focus_changed(focused) {
                        // The field editor is attached while the field
                        // takes focus: look again once it has.
                        let later = Rc::clone(&shared);
                        shared.events.defer(move || {
                            later.line_focus_changed(true);
                        });
                    }
                })));
            }
            Control::Area { view, .. } => {
                let (shared, view) = (Rc::clone(&self.shared), Weak::from_retained(view));
                container.set_focus_listener(Some(Rc::new(move |focused| {
                    if !focused || !shared.config.borrow().select_on_focus {
                        return;
                    }
                    let view = view.clone();
                    // After AppKit placed the caret for the click that focused it.
                    shared.events.defer(move || {
                        if let Some(view) = view.load() {
                            // SAFETY: `selectAll:` accepts a nil sender.
                            unsafe { view.selectAll(None) };
                        }
                    });
                })));
            }
        }
    }

    /// The view placed inside the node's view.
    pub fn view(&self) -> Retained<NSView> {
        match &self.control {
            Control::Line(field) => Retained::into_super(Retained::into_super(field.clone())),
            Control::Area { scroll, .. } => Retained::into_super(scroll.clone()),
        }
    }

    /// The view that takes keyboard focus and speaks for the control.
    pub fn focus_target(&self) -> Retained<NSView> {
        match &self.control {
            Control::Line(field) => Retained::into_super(Retained::into_super(field.clone())),
            Control::Area { view, .. } => Retained::into_super(Retained::into_super(view.clone())),
        }
    }

    /// Gives the control keyboard focus.
    pub fn focus(&self) {
        let target = self.focus_target();
        let Some(window) = target.window() else {
            return;
        };
        if !window.makeFirstResponder(Some(&target)) {
            return;
        }
        // A field focused by code starts with everything selected.
        let collapse = !self.shared.config.borrow().select_on_focus;
        if let Control::Line(field) = &self.control
            && collapse
            && let Some(editor) = field.currentEditor()
        {
            self.shared.updating.set(true);
            editor.setSelectedRange(NSRange::new(editor.string().length(), 0));
            self.shared.updating.set(false);
        }
    }

    /// Places the control inside the box, within padding and borders.
    pub fn layout(&mut self, size: Size) {
        self.size = size;
        let (insets, vertical) = {
            let config = self.shared.config.borrow();
            (config.insets, config.vertical)
        };
        let width = (size.width - insets.horizontal()).max(0.0);
        let available = (size.height - insets.vertical()).max(0.0);
        match &self.control {
            Control::Line(field) => {
                // A field draws its line at its top: it is sized to one
                // line and placed where the alignment asks.
                let line = text::line_height(&text::font(&self.shared.config.borrow().font));
                let natural = field
                    .cell()
                    .map_or(0.0, |cell| cell.cellSize().height as f32)
                    .max(line)
                    .ceil();
                let height = natural.min(available);
                let top = insets.top
                    + match vertical {
                        VerticalAlign::Top => 0.0,
                        VerticalAlign::Center => ((available - height) / 2.0).floor(),
                        VerticalAlign::Bottom => available - height,
                    };
                field.setFrame(cg_rect(pam_desktop_ui::geometry::Rect::new(
                    insets.left,
                    top,
                    width,
                    height,
                )));
            }
            Control::Area { scroll, view } => {
                scroll.setFrame(cg_rect(pam_desktop_ui::geometry::Rect::new(
                    insets.left,
                    insets.top,
                    width,
                    available,
                )));
                self.shared.report_content_size(view);
            }
        }
    }

    pub fn configure(&mut self, config: &InputConfig, container: &PamView, parent: &NSView) {
        *self.shared.config.borrow_mut() = config.clone();
        container.set_context_menu_hidden(config.context_menu_hidden);

        // Masking is a different class of field.
        if let Control::Line(field) = &self.control
            && config.secure != self.secure
        {
            let value = field.stringValue();
            // The caret (or selection) of a focused field survives the swap.
            let selection = field.currentEditor().map(|editor| editor.selectedRange());
            let replacement = make_line(self.mtm, config.secure);
            replacement.setStringValue(&value);
            parent.addSubview(&replacement);
            field.removeFromSuperview();
            self.control = Control::Line(replacement.clone());
            self.secure = config.secure;
            self.attach(container);
            if let Some(range) = selection {
                self.shared.keep_selection.set(true);
                self.focus();
                let restore = move |field: &NSTextField| {
                    let Some(editor) = field.currentEditor() else {
                        return false;
                    };
                    let length = editor.string().length();
                    let start = range.location.min(length);
                    let end = (range.location + range.length).min(length);
                    editor.setSelectedRange(NSRange::new(start, end - start));
                    true
                };
                if restore(&replacement) {
                    // AppKit selects the whole value of a field that takes
                    // focus once the current event is done: put the caret
                    // back after that too.
                    let field = Weak::from_retained(&replacement);
                    self.shared.events.defer(move || {
                        if let Some(field) = field.load() {
                            restore(&field);
                        }
                    });
                } else {
                    self.shared.keep_selection.set(false);
                }
            }
        }

        let font = text::font(&config.font);
        let color = config.color.map_or_else(NSColor::labelColor, ns_color);
        let plain = matches!(config.purpose, InputPurpose::Text | InputPurpose::Search);
        match &self.control {
            Control::Line(field) => {
                field.setFont(Some(&font));
                field.setTextColor(Some(&color));
                field.setAlignment(text::alignment(config.align));
                field.setEditable(config.editable);
                field.setSelectable(true);
                field.setEnabled(config.enabled);
                field.setAutomaticTextCompletionEnabled(plain && !config.secure);
                if config.placeholder.is_empty() {
                    field.setPlaceholderAttributedString(None);
                } else {
                    // Without an authored colour the hint is the system's.
                    let hint = placeholder_color(config);
                    let attributes = self
                        .shared
                        .text_attributes(Some(hint.unwrap_or(Color::rgba(128, 128, 128, 200))));
                    if hint.is_none() {
                        // SAFETY: the attribute name is an immutable string
                        // exported by AppKit and its value is a color.
                        unsafe {
                            attributes.insert(
                                NSForegroundColorAttributeName,
                                any(&*NSColor::placeholderTextColor()),
                            )
                        };
                    }
                    // SAFETY: the attributes are the standard text attributes built above.
                    let placeholder = unsafe {
                        NSAttributedString::initWithString_attributes(
                            NSAttributedString::alloc(),
                            &ns_string(&config.placeholder),
                            Some(&attributes),
                        )
                    };
                    field.setPlaceholderAttributedString(Some(&placeholder));
                }
                // A field that is being edited takes the new style at once.
                if let Some(editor) = field
                    .currentEditor()
                    .and_then(|editor| editor.downcast::<NSTextView>().ok())
                {
                    self.shared.style_editor(&editor);
                }
                self.shared.restyle_line(field);
            }
            Control::Area { view, .. } => {
                view.setFont(Some(&font));
                view.setTextColor(Some(&color));
                view.setAlignment(text::alignment(config.align));
                view.setEditable(config.editable && config.enabled);
                view.setSelectable(config.enabled);
                view.setAutomaticSpellingCorrectionEnabled(config.spellcheck && plain);
                view.setAutomaticTextReplacementEnabled(plain);
                view.setAutomaticQuoteSubstitutionEnabled(plain);
                view.setAutomaticDashSubstitutionEnabled(plain);
                view.setInsertionPointColor(Some(&color));
                // SAFETY: the attributes are the standard text attributes;
                // the default selection uses the system's color.
                unsafe {
                    view.setTypingAttributes(&self.shared.text_attributes(None));
                    if config.selection_color.is_none() {
                        let key: &NSString = NSBackgroundColorAttributeName;
                        let attributes: Retained<NSDictionary<NSAttributedStringKey, AnyObject>> =
                            NSDictionary::from_slices(
                                &[key],
                                &[any(&*NSColor::selectedTextBackgroundColor())],
                            );
                        view.setSelectedTextAttributes(&attributes);
                    }
                }
                self.shared.style_editor(view);
                self.shared.refresh_placeholder(view.string().length() == 0);
            }
        }
        self.layout(self.size);
    }

    pub fn set_text(&self, value: &str, selection: Option<(u32, u32)>) {
        let shared = &self.shared;
        shared.updating.set(true);
        let text = ns_string(value);
        let length = text.length();
        let range = match selection {
            Some((start, end)) => {
                let start = (start as usize).min(length);
                NSRange::new(start, (end as usize).min(length).saturating_sub(start))
            }
            None => NSRange::new(length, 0),
        };
        match &self.control {
            Control::Line(field) => {
                if *field.stringValue() != *text {
                    field.setStringValue(&text);
                }
                match field.currentEditor() {
                    Some(editor) => {
                        editor.setSelectedRange(range);
                        if let Some(editor) = editor.downcast_ref::<NSTextView>() {
                            shared.style_editor(editor);
                        }
                    }
                    None => shared.restyle_line(field),
                }
            }
            Control::Area { view, .. } => {
                if *view.string() != *text {
                    view.setString(&text);
                    // SAFETY: the attributes are the standard text attributes.
                    unsafe { view.setTypingAttributes(&shared.text_attributes(None)) };
                }
                view.setSelectedRange(range);
                shared.refresh_placeholder(length == 0);
                shared.report_content_size(view);
            }
        }
        shared.updating.set(false);
    }

    pub fn set_selection(&self, start: u32, end: u32) {
        let editor: Option<Retained<NSText>> = match &self.control {
            Control::Line(field) => field.currentEditor(),
            Control::Area { view, .. } => Some(Retained::into_super(view.clone())),
        };
        let Some(editor) = editor else { return };
        let length = editor.string().length();
        let start = (start as usize).min(length);
        self.shared.updating.set(true);
        editor.setSelectedRange(NSRange::new(
            start,
            (end as usize).min(length).saturating_sub(start),
        ));
        self.shared.updating.set(false);
    }

    /// Detaches the control from the node's view and from its observers.
    pub fn destroy(&self) {
        // SAFETY: removes the registrations made in `attach`; clearing the
        // delegate is always valid.
        unsafe {
            NSNotificationCenter::defaultCenter().removeObserver(&self.delegate);
            match &self.control {
                Control::Line(field) => field.setDelegate(None),
                Control::Area { view, .. } => view.setDelegate(None),
            }
        }
        if let Some(container) = self.shared.container.load() {
            container.set_focus_listener(None);
        }
        self.view().removeFromSuperview();
    }
}

/// The colour of the hint: the authored one, else the text colour at 45%,
/// like the GTK backend; `None` leaves it to the system.
fn placeholder_color(config: &InputConfig) -> Option<Color> {
    config
        .placeholder_color
        .or_else(|| config.color.map(|color| color.with_alpha(0.45)))
}
