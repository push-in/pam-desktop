//! Typed access to the property bag of a wire node.

use std::collections::BTreeMap;

use pam_desktop_wire::{PropKey, PropValue};

use crate::color::Color;
use crate::geometry::Insets;

pub type PropMap = BTreeMap<PropKey, PropValue>;

/// Read-only view with the coercions every host applies (integers are valid
/// decimals, `0`/`1` are valid flags).
#[derive(Clone, Copy)]
pub struct Props<'a>(pub &'a PropMap);

impl<'a> Props<'a> {
    #[must_use]
    pub fn has(&self, key: PropKey) -> bool {
        self.0.contains_key(&key)
    }

    #[must_use]
    pub fn raw(&self, key: PropKey) -> Option<&'a PropValue> {
        self.0.get(&key)
    }

    #[must_use]
    pub fn number(&self, key: PropKey) -> Option<f32> {
        match self.0.get(&key)? {
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
        match self.0.get(&key)? {
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
        match self.0.get(&key)? {
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
        match self.0.get(&key)? {
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
        match self.0.get(&key)? {
            PropValue::Bytes(value) => Some(value.as_slice()),
            PropValue::String(value) => Some(value.as_bytes()),
            _ => None,
        }
    }

    #[must_use]
    pub fn color(&self, key: PropKey) -> Option<Color> {
        match self.0.get(&key)? {
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
            self.number(PropKey::PaddingLeft).unwrap_or(horizontal).max(0.0),
            self.number(PropKey::PaddingTop).unwrap_or(vertical).max(0.0),
            self.number(PropKey::PaddingRight).unwrap_or(horizontal).max(0.0),
            self.number(PropKey::PaddingBottom).unwrap_or(vertical).max(0.0),
        )
    }
}
