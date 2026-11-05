//! Text inputs: the controlled-value protocol between PHP and a native text
//! control.
//!
//! The control owns the text while the user types. PHP hears about edits
//! according to the input's sync mode, and values coming back from PHP are
//! compared against what was sent so a stale echo never overwrites newer
//! keystrokes.

use std::collections::VecDeque;

use pam_desktop_wire::{NodeKind, PropKey};

use crate::animator::Timer;
use crate::backend::{Capitalization, InputConfig, InputEvent, InputPurpose, NodeId, VerticalAlign};
use crate::context::{event_kind, Ctx, HostCommand};
use crate::geometry::Insets;
use crate::input_format::{system_locale, Format};
use crate::props::Props;
use crate::scene::{Change, IdMap};
use crate::text::{char_to_utf16_offset, effective_font_scale, utf16_len, utf16_to_char_offset, FontSpec, TextAlign};
use crate::wiremap::WireMap;

const SYNC_NATIVE: i64 = 1;
const SYNC_DEBOUNCED: i64 = 2;
const SYNC_IMMEDIATE: i64 = 3;
const SYNC_ON_BLUR: i64 = 4;
const SYNC_ON_SUBMIT: i64 = 5;

const SUBMIT_BLUR_AND_SUBMIT: i64 = 2;
const SUBMIT_NEWLINE: i64 = 3;

/// Values sent to PHP that may still come back as an echo.
const IN_FLIGHT_LIMIT: usize = 64;
const IN_FLIGHT_WINDOW_MS: u64 = 3000;

#[derive(Debug, Default)]
struct Input {
    /// Text currently in the native control.
    text: String,
    in_flight: VecDeque<(String, u64)>,
    focused: bool,
    /// An edit PHP has not heard about yet.
    pending: bool,
    last_sent: Option<String>,
    config: Option<InputConfig>,
    autofocused: bool,
    reconfigure: bool,
    value_changed: bool,
    selection_changed: bool,
}

#[derive(Default)]
pub struct Inputs {
    inputs: IdMap<Input>,
    dirty: bool,
}

fn format_of(props: Props<'_>) -> Format {
    match props.integer_or(PropKey::InputFormat, 1) {
        2 => Format::Pattern {
            pattern: props.text_or_empty(PropKey::InputFormatPattern).chars().take(128).collect(),
            slot: props.text(PropKey::InputFormatPlaceholder).and_then(|slot| slot.chars().next()).unwrap_or('#'),
        },
        3 => Format::Currency {
            prefix: props.text_or_empty(PropKey::InputFormatPrefix).chars().take(16).collect(),
            suffix: props.text_or_empty(PropKey::InputFormatSuffix).chars().take(16).collect(),
            decimals: props.integer_or(PropKey::InputFormatDecimalDigits, 2).clamp(0, 6) as u8,
            locale: props.text(PropKey::InputFormatLocale).filter(|locale| !locale.is_empty()).map_or_else(system_locale, str::to_owned),
        },
        _ => Format::None,
    }
}

/// The controlled value of an input node (`Value`, or `Text` as a fallback).
fn authored_value<'a>(props: Props<'a>) -> Option<&'a str> {
    props.text(PropKey::Value).or_else(|| props.text(PropKey::Text))
}

fn resolve_config(props: Props<'_>, multiline: bool, border: [f32; 4], font_scale: f32) -> InputConfig {
    let padding = props.padding();
    let format = props.integer_or(PropKey::InputFormat, 1);
    let purpose = match props.integer_or(PropKey::InputMode, 1) {
        3 => InputPurpose::Decimal,
        4 => InputPurpose::Number,
        5 => InputPurpose::Phone,
        7 => InputPurpose::Email,
        8 => InputPurpose::Url,
        mode => match props.integer_or(PropKey::KeyboardType, 1) {
            2 => InputPurpose::Email,
            3 => InputPurpose::Number,
            4 => InputPurpose::Phone,
            5 => InputPurpose::Decimal,
            6 => InputPurpose::Url,
            _ if mode == 6 => InputPurpose::Search,
            _ => InputPurpose::Text,
        },
    };
    let submit_behavior = props.integer_or(PropKey::InputSubmitBehavior, if multiline { SUBMIT_NEWLINE } else { SUBMIT_BLUR_AND_SUBMIT });
    InputConfig {
        placeholder: props.text_or_empty(PropKey::Placeholder).to_owned(),
        placeholder_color: props.color(PropKey::PlaceholderColor),
        font: FontSpec {
            family: props.text_or_empty(PropKey::FontFamily).to_owned(),
            size: props.number_or(PropKey::FontSize, 14.0).max(1.0) * effective_font_scale(props, font_scale),
            weight: props.integer_or(PropKey::FontWeight, 400).clamp(1, 1000) as u16,
            italic: props.integer(PropKey::FontStyle) == Some(2),
            features: props.text_or_empty(PropKey::FontFeatureSettings).to_owned(),
        },
        color: props.color(PropKey::TextColor),
        align: match props.integer(PropKey::TextAlign) {
            Some(2) => TextAlign::Center,
            Some(3) => TextAlign::End,
            _ => TextAlign::Start,
        },
        vertical: match props.integer_or(PropKey::InputTextAlignVertical, 1) {
            2 => VerticalAlign::Top,
            3 => VerticalAlign::Center,
            4 => VerticalAlign::Bottom,
            _ if multiline => VerticalAlign::Top,
            _ => VerticalAlign::Center,
        },
        insets: Insets::new(padding.left + border[0], padding.top + border[1], padding.right + border[2], padding.bottom + border[3]),
        letter_spacing: props.number_or(PropKey::LetterSpacing, 0.0),
        secure: props.flag_or(PropKey::Secure, false) && !multiline,
        editable: props.flag_or(PropKey::InputEditable, true),
        enabled: props.flag_or(PropKey::Enabled, true),
        max_length: props.integer(PropKey::MaxLength).filter(|length| *length >= 0).map(|length| length.min(i64::from(u32::MAX)) as u32),
        purpose,
        capitalization: match props.integer_or(PropKey::InputAutoCapitalize, 2) {
            1 => Capitalization::None,
            3 => Capitalization::Words,
            4 => Capitalization::Characters,
            _ => Capitalization::Sentences,
        },
        // Masks own the text: corrections would fight them.
        spellcheck: format == 1 && props.flag_or(PropKey::InputAutoCorrect, true),
        selection_color: props.color(PropKey::SelectionColor),
        caret_color: props.color(PropKey::InputCursorColor).or_else(|| props.color(PropKey::SelectionColor)),
        caret_hidden: props.flag_or(PropKey::InputCaretHidden, false),
        context_menu_hidden: props.flag_or(PropKey::InputContextMenuHidden, false),
        select_on_focus: props.flag_or(PropKey::InputSelectTextOnFocus, false),
        submit_on_enter: submit_behavior != SUBMIT_NEWLINE,
    }
}

impl Inputs {
    pub fn on_change(&mut self, ctx: &mut Ctx<'_>, change: Change) {
        match change {
            Change::Created(id, NodeKind::Input) => {
                self.inputs.insert(id, Input { reconfigure: true, value_changed: true, selection_changed: true, ..Input::default() });
                self.dirty = true;
            }
            Change::Removed(id, NodeKind::Input) => {
                self.inputs.remove(&id);
                ctx.timers.cancel(Timer::InputDebounce(id));
            }
            Change::Updated(id, key) => {
                let Some(input) = self.inputs.get_mut(&id) else { return };
                match key {
                    PropKey::Value | PropKey::Text => input.value_changed = true,
                    PropKey::InputSelectionStart | PropKey::InputSelectionEnd => input.selection_changed = true,
                    PropKey::InputFormat
                    | PropKey::InputFormatPattern
                    | PropKey::InputFormatPlaceholder
                    | PropKey::InputFormatPrefix
                    | PropKey::InputFormatSuffix
                    | PropKey::InputFormatDecimalDigits
                    | PropKey::InputFormatLocale => {
                        // A new mask re-formats the current text.
                        input.value_changed = true;
                        input.reconfigure = true;
                    }
                    PropKey::AutoFocus => {
                        input.autofocused = false;
                        input.reconfigure = true;
                    }
                    _ => input.reconfigure = true,
                }
                self.dirty = true;
            }
            _ => {}
        }
    }

    pub fn after_commit(&mut self, ctx: &mut Ctx<'_>) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let ids: Vec<NodeId> = self
            .inputs
            .iter()
            .filter(|(_, input)| input.reconfigure || input.value_changed || input.selection_changed || !input.autofocused)
            .map(|(id, _)| *id)
            .collect();
        for id in ids {
            let Some(node) = ctx.scene.node(id) else { continue };
            let props = node.props();
            let multiline = props.flag_or(PropKey::Multiline, false);
            let config = resolve_config(props, multiline, node.decoration.border.widths, ctx.scene.environment.font_scale);
            let selection = props.integer(PropKey::InputSelectionStart).map(|start| {
                let start = start.max(0) as u32;
                let end = props.integer(PropKey::InputSelectionEnd).map_or(start, |end| (end.max(0) as u32).max(start));
                (start, end)
            });
            let wants_focus = props.flag_or(PropKey::AutoFocus, false);
            let Some(input) = self.inputs.get_mut(&id) else { continue };
            if std::mem::take(&mut input.reconfigure) && input.config.as_ref() != Some(&config) {
                ctx.commands.push(HostCommand::InputConfig { id, config: Box::new(config.clone()) });
                input.config = Some(config);
            }
            if std::mem::take(&mut input.value_changed) {
                self.apply_value(ctx, id);
            }
            let Some(input) = self.inputs.get_mut(&id) else { continue };
            if std::mem::take(&mut input.selection_changed) {
                if let Some((start, end)) = selection {
                    let length = utf16_len(&input.text);
                    ctx.commands.push(HostCommand::InputSelection { id, start: start.min(length), end: end.min(length) });
                }
            }
            // AutoFocus waits until the field is actually on screen (a hidden
            // route or a closed modal becoming visible retries it).
            if !input.autofocused {
                if !wants_focus {
                    input.autofocused = true;
                } else if ctx.scene.is_effectively_visible(id) {
                    input.autofocused = true;
                    ctx.commands.push(HostCommand::Focus(Some(id)));
                } else {
                    self.dirty = true;
                }
            }
        }
    }

    /// A route or modal became visible: pending auto-focus requests retry.
    pub fn retry_autofocus(&mut self) {
        if self.inputs.values().any(|input| !input.autofocused) {
            self.dirty = true;
        }
    }

    /// A value arrived from PHP.
    fn apply_value(&mut self, ctx: &mut Ctx<'_>, id: NodeId) {
        let Some(node) = ctx.scene.node(id) else { return };
        let props = node.props();
        let Some(authored) = authored_value(props) else { return };
        let value = format_of(props).format(authored);
        let Some(input) = self.inputs.get_mut(&id) else { return };
        if input.focused {
            while input.in_flight.front().is_some_and(|(_, sent)| ctx.now_ms.saturating_sub(*sent) > IN_FLIGHT_WINDOW_MS) {
                input.in_flight.pop_front();
            }
            if value == input.text {
                input.in_flight.clear();
                return;
            }
            if let Some(position) = input.in_flight.iter().position(|(sent, _)| *sent == value) {
                // An echo of something typed earlier: newer keystrokes win.
                input.in_flight.drain(..=position);
                return;
            }
            // An authored change (cleared after sending, replaced by a suggestion).
            input.in_flight.clear();
        } else if value == input.text {
            return;
        }
        let caret = if input.focused { utf16_len(&value) } else { 0 };
        input.text.clone_from(&value);
        input.last_sent = Some(value.clone());
        input.pending = false;
        ctx.timers.cancel(Timer::InputDebounce(id));
        ctx.commands.push(HostCommand::InputText { id, text: value, selection: Some((caret, caret)) });
    }

    fn send_change(&mut self, ctx: &mut Ctx<'_>, id: NodeId) {
        let Some(input) = self.inputs.get_mut(&id) else { return };
        input.pending = false;
        ctx.timers.cancel(Timer::InputDebounce(id));
        if input.last_sent.as_deref() == Some(input.text.as_str()) {
            return;
        }
        let text = input.text.clone();
        input.last_sent = Some(text.clone());
        if input.in_flight.back().is_none_or(|(sent, _)| *sent != text) {
            input.in_flight.push_back((text.clone(), ctx.now_ms));
            if input.in_flight.len() > IN_FLIGHT_LIMIT {
                input.in_flight.pop_front();
            }
        }
        ctx.emit_if(id, PropKey::OnChange, event_kind::CHANGE, text.into_bytes());
    }

    pub fn on_event(&mut self, ctx: &mut Ctx<'_>, id: NodeId, event: InputEvent) {
        let Some(node) = ctx.scene.node(id) else { return };
        let props = node.props();
        let multiline = props.flag_or(PropKey::Multiline, false);
        let sync = props.integer_or(PropKey::InputSyncMode, SYNC_DEBOUNCED);
        let debounce = props.integer_or(PropKey::InputDebounceMs, 48).clamp(0, 5000) as u64;
        let submit_behavior = props.integer_or(PropKey::InputSubmitBehavior, if multiline { SUBMIT_NEWLINE } else { SUBMIT_BLUR_AND_SUBMIT });
        let format = format_of(props);
        let Some(input) = self.inputs.get_mut(&id) else { return };
        match event {
            InputEvent::Edited { text, selection } => {
                let caret = utf16_to_char_offset(&text, selection.1) as usize;
                let (formatted, caret) = format.edit(&input.text, &text, caret);
                if formatted != text {
                    let caret = char_to_utf16_offset(&formatted, caret as u32);
                    ctx.commands.push(HostCommand::InputText { id, text: formatted.clone(), selection: Some((caret, caret)) });
                }
                if formatted == input.text {
                    return;
                }
                input.text = formatted;
                match sync {
                    SYNC_IMMEDIATE => self.send_change(ctx, id),
                    SYNC_DEBOUNCED if debounce == 0 => self.send_change(ctx, id),
                    SYNC_DEBOUNCED => {
                        input.pending = true;
                        ctx.timers.schedule(Timer::InputDebounce(id), ctx.now_ms + debounce);
                    }
                    _ => input.pending = true,
                }
            }
            InputEvent::SelectionChanged { start, end } => {
                let payload = WireMap::new().with("start", i64::from(start)).with("end", i64::from(end)).encode();
                ctx.emit_if(id, PropKey::OnInputSelectionChange, event_kind::INPUT_SELECTION_CHANGE, payload);
            }
            InputEvent::KeyPressed { key } => {
                let key: String = key.chars().take(64).collect();
                ctx.emit_if(id, PropKey::OnInputKeyPress, event_kind::INPUT_KEY_PRESS, WireMap::new().with("key", key).encode());
            }
            InputEvent::Submitted => {
                if submit_behavior == SUBMIT_NEWLINE {
                    return;
                }
                if input.pending && matches!(sync, SYNC_NATIVE | SYNC_DEBOUNCED | SYNC_ON_SUBMIT) {
                    self.send_change(ctx, id);
                }
                let text = self.inputs.get(&id).map(|input| input.text.clone()).unwrap_or_default();
                ctx.emit_if(id, PropKey::OnSubmit, event_kind::SUBMIT, text.into_bytes());
                if submit_behavior == SUBMIT_BLUR_AND_SUBMIT {
                    ctx.commands.push(HostCommand::Focus(None));
                }
            }
            InputEvent::ContentSizeChanged { size } => {
                let payload = WireMap::new().with("width", size.width).with("height", size.height).encode();
                ctx.emit_if(id, PropKey::OnInputContentSizeChange, event_kind::INPUT_CONTENT_SIZE_CHANGE, payload);
            }
        }
    }

    /// Returns whether `id` is an input (focus events of other nodes are not ours).
    pub fn on_focus(&mut self, ctx: &mut Ctx<'_>, id: NodeId, focused: bool) -> bool {
        let Some(input) = self.inputs.get_mut(&id) else { return false };
        if input.focused == focused {
            return true;
        }
        input.focused = focused;
        if focused {
            ctx.emit_if(id, PropKey::OnFocus, event_kind::FOCUS, Vec::new());
            return true;
        }
        let sync = ctx.scene.node(id).map_or(SYNC_DEBOUNCED, |node| node.props().integer_or(PropKey::InputSyncMode, SYNC_DEBOUNCED));
        if input.pending && matches!(sync, SYNC_NATIVE | SYNC_DEBOUNCED | SYNC_ON_BLUR) {
            self.send_change(ctx, id);
        }
        ctx.emit_if(id, PropKey::OnBlur, event_kind::BLUR, Vec::new());
        let Some(input) = self.inputs.get_mut(&id) else { return true };
        input.in_flight.clear();
        let text = input.text.clone();
        ctx.emit_if(id, PropKey::OnInputEndEditing, event_kind::INPUT_END_EDITING, text.into_bytes());
        // A value PHP sent while the user was typing is now authoritative.
        self.apply_value(ctx, id);
        true
    }

    pub fn on_timer(&mut self, ctx: &mut Ctx<'_>, id: NodeId) {
        if self.inputs.get(&id).is_some_and(|input| input.pending) {
            self.send_change(ctx, id);
        }
    }

    /// A press elsewhere must observe exactly what the focused editor shows:
    /// inputs that keep keystrokes on the host flush before the press.
    pub fn flush_before_press(&mut self, ctx: &mut Ctx<'_>) {
        let pending: Vec<NodeId> = self.inputs.iter().filter(|(_, input)| input.focused && input.pending).map(|(id, _)| *id).collect();
        for id in pending {
            let sync = ctx.scene.node(id).map_or(SYNC_DEBOUNCED, |node| node.props().integer_or(PropKey::InputSyncMode, SYNC_DEBOUNCED));
            if matches!(sync, SYNC_NATIVE | SYNC_DEBOUNCED) {
                self.send_change(ctx, id);
            }
        }
    }

    /// The text the native control shows (view-ref commands, tests).
    #[must_use]
    pub fn text(&self, id: NodeId) -> Option<&str> {
        self.inputs.get(&id).map(|input| input.text.as_str())
    }

    /// Replaces the text from the host side (`view-ref.clear`).
    pub fn set_text(&mut self, ctx: &mut Ctx<'_>, id: NodeId, text: &str) {
        let Some(input) = self.inputs.get_mut(&id) else { return };
        if input.text == text {
            return;
        }
        input.text = text.to_owned();
        ctx.commands.push(HostCommand::InputText { id, text: text.to_owned(), selection: None });
        self.send_change(ctx, id);
    }
}
