//! Typed access to the property bag of a wire node.

use std::collections::BTreeMap;

use pam_desktop_wire::{PropKey, PropValue};

use crate::color::Color;
use crate::geometry::Insets;

pub type PropMap = BTreeMap<PropKey, PropValue>;

/// Read-only view with the coercions every host applies (integers are valid
/// decimals, `0`/`1` are valid flags). An optional overlay (interaction state
/// styles, presentation chrome) wins over the node's own properties.
#[derive(Clone, Copy)]
pub struct Props<'a> {
    base: &'a PropMap,
    /// Checked first to last; the first layer that has the key wins.
    overlays: [Option<&'a PropMap>; 2],
}

impl<'a> Props<'a> {
    #[must_use]
    pub const fn new(base: &'a PropMap) -> Self {
        Self {
            base,
            overlays: [None, None],
        }
    }

    #[must_use]
    pub const fn with_overlays(base: &'a PropMap, overlays: [Option<&'a PropMap>; 2]) -> Self {
        Self { base, overlays }
    }

    #[must_use]
    pub fn has(&self, key: PropKey) -> bool {
        self.raw(key).is_some()
    }

    #[must_use]
    pub fn raw(&self, key: PropKey) -> Option<&'a PropValue> {
        for overlay in self.overlays.iter().flatten() {
            if let Some(value) = overlay.get(&key) {
                return Some(value);
            }
        }
        self.base.get(&key)
    }

    #[must_use]
    pub fn number(&self, key: PropKey) -> Option<f32> {
        match self.raw(key)? {
            PropValue::Integer(value) => Some(*value as f32),
            PropValue::Float(value) if value.is_finite() => Some(*value as f32),
            _ => None,
        }
    }

    #[must_use]
    pub fn number_or(&self, key: PropKey, default: f32) -> f32 {
        self.number(key).unwrap_or(default)
    }

    #[must_use]
    pub fn integer(&self, key: PropKey) -> Option<i64> {
        match self.raw(key)? {
            PropValue::Integer(value) => Some(*value),
            PropValue::Float(value) if value.is_finite() => Some(*value as i64),
            PropValue::Boolean(value) => Some(i64::from(*value)),
            _ => None,
        }
    }

    #[must_use]
    pub fn integer_or(&self, key: PropKey, default: i64) -> i64 {
        self.integer(key).unwrap_or(default)
    }

    #[must_use]
    pub fn flag(&self, key: PropKey) -> Option<bool> {
        match self.raw(key)? {
            PropValue::Boolean(value) => Some(*value),
            PropValue::Integer(value) => Some(*value != 0),
            _ => None,
        }
    }

    #[must_use]
    pub fn flag_or(&self, key: PropKey, default: bool) -> bool {
        self.flag(key).unwrap_or(default)
    }

    #[must_use]
    pub fn text(&self, key: PropKey) -> Option<&'a str> {
        match self.raw(key)? {
            PropValue::String(value) => Some(value.as_str()),
            _ => None,
        }
    }

    #[must_use]
    pub fn text_or_empty(&self, key: PropKey) -> &'a str {
        self.text(key).unwrap_or("")
    }

    #[must_use]
    pub fn bytes(&self, key: PropKey) -> Option<&'a [u8]> {
        match self.raw(key)? {
            PropValue::Bytes(value) => Some(value.as_slice()),
            PropValue::String(value) => Some(value.as_bytes()),
            _ => None,
        }
    }

    #[must_use]
    pub fn color(&self, key: PropKey) -> Option<Color> {
        match self.raw(key)? {
            PropValue::Integer(value) => Some(Color::from_wire(*value)),
            _ => None,
        }
    }

    /// Padding of the node's own box (shorthands resolved like the engine).
    #[must_use]
    pub fn padding(&self) -> Insets {
        let all = self.number(PropKey::Padding).unwrap_or(0.0);
        let horizontal = self.number(PropKey::PaddingHorizontal).unwrap_or(all);
        let vertical = self.number(PropKey::PaddingVertical).unwrap_or(all);
        Insets::new(
            self.number(PropKey::PaddingLeft)
                .unwrap_or(horizontal)
                .max(0.0),
            self.number(PropKey::PaddingTop)
                .unwrap_or(vertical)
                .max(0.0),
            self.number(PropKey::PaddingRight)
                .unwrap_or(horizontal)
                .max(0.0),
            self.number(PropKey::PaddingBottom)
                .unwrap_or(vertical)
                .max(0.0),
        )
    }
}
