//! The test driver (`PAM_DESKTOP_SCRIPT=steps.json`): runs a script of
//! user actions and checks against the live window, then quits with the
//! result. It works on every backend: actions are fed to the core as the
//! backend would report them.
//!
//! A step is one JSON object:
//!
//! - `{"wait": "selector"}` until it is on screen (`timeout`, 10 s by default)
//! - `{"click": "selector"}`, `{"doubleClick": …}`, `{"hover": …}`
//! - `{"type": "text"}` into the focused field, or `{"type": "text", "into": "selector"}`
//! - `{"submit": "selector"}`: Enter in a field (its `on:submit`)
//! - `{"key": "Ctrl+P"}` (a registered shortcut, or a key for the focused element)
//! - `{"expect": "selector"}` and `{"expectNot": "selector"}` (waits up to `timeout`)
//! - `{"sleep": 300}`
//!
//! A selector is `#testId`, `@accessibility label`, or text the element shows.

use std::sync::atomic::{AtomicBool, Ordering};

use pam_desktop_wire::{NodeKind, PropKey};

use crate::backend::NodeId;
use crate::geometry::Point;
use crate::scene::Scene;

static FAILED: AtomicBool = AtomicBool::new(false);

/// Marks the script as failed.
pub fn fail() {
    FAILED.store(true, Ordering::SeqCst);
}

/// Whether a script ran and failed (the host turns it into its exit code).
#[must_use]
pub fn failed() -> bool {
    FAILED.load(Ordering::SeqCst)
}

#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    Wait(String),
    Click(String, u8),
    Hover(String),
    Type(String, Option<String>),
    Submit(String),
    Key(String),
    Expect(String, bool),
    Sleep(u64),
}

#[derive(Debug)]
pub struct Driver {
    steps: Vec<(Step, u64)>,
    index: usize,
    started: Option<u64>,
}

/// What the runtime should do for the current step.
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// Check again later.
    Wait,
    Click(NodeId, Point, u8),
    Hover(Point),
    Type(Option<NodeId>, String),
    Submit(NodeId),
    Key(String),
    /// The step is done: go on.
    Next,
    Done,
    Fail(String),
}

impl Driver {
    /// Reads a script file.
    ///
    /// # Errors
    ///
    /// When the file cannot be read or holds no valid steps.
    pub fn load(path: &std::path::Path) -> Result<Self, String> {
        let json = std::fs::read_to_string(path)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        Self::parse(&json)
    }

    /// # Errors
    ///
    /// When a step is not one the driver knows.
    pub fn parse(json: &str) -> Result<Self, String> {
        let value: serde_json::Value =
            serde_json::from_str(json).map_err(|error| error.to_string())?;
        let entries = value
            .as_array()
            .or_else(|| value.get("steps").and_then(serde_json::Value::as_array))
            .ok_or("a script is a list of steps")?;
        let mut steps = Vec::new();
        for (position, entry) in entries.iter().enumerate() {
            let text = |key: &str| {
                entry
                    .get(key)
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            };
            let timeout = entry
                .get("timeout")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(10_000);
            let step = if let Some(selector) = text("wait") {
                Step::Wait(selector)
            } else if let Some(selector) = text("click") {
                Step::Click(selector, 1)
            } else if let Some(selector) = text("doubleClick") {
                Step::Click(selector, 2)
            } else if let Some(selector) = text("hover") {
                Step::Hover(selector)
            } else if let Some(typed) = text("type") {
                Step::Type(typed, text("into"))
            } else if let Some(selector) = text("submit") {
                Step::Submit(selector)
            } else if let Some(key) = text("key") {
                Step::Key(key)
            } else if let Some(selector) = text("expect") {
                Step::Expect(selector, true)
            } else if let Some(selector) = text("expectNot") {
                Step::Expect(selector, false)
            } else if let Some(ms) = entry.get("sleep").and_then(serde_json::Value::as_u64) {
                Step::Sleep(ms.min(60_000))
            } else {
                return Err(format!(
                    "step {} is not one the driver knows: {entry}",
                    position + 1
                ));
            };
            steps.push((step, timeout));
        }
        Ok(Self {
            steps,
            index: 0,
            started: None,
        })
    }

    #[must_use]
    pub fn describe(&self) -> String {
        self.steps
            .get(self.index)
            .map_or_else(String::new, |(step, _)| {
                format!("step {} {step:?}", self.index + 1)
            })
    }

    /// The action for the current step at time `now` (ms).
    pub fn poll(&mut self, scene: &Scene, now: u64) -> Action {
        let Some((step, timeout)) = self.steps.get(self.index).cloned() else {
            return Action::Done;
        };
        let started = *self.started.get_or_insert(now);
        let late = now.saturating_sub(started) > timeout;
        let action = match &step {
            Step::Sleep(ms) => {
                if now.saturating_sub(started) >= *ms {
                    Action::Next
                } else {
                    Action::Wait
                }
            }
            Step::Wait(selector) | Step::Expect(selector, true) => match find(scene, selector) {
                Some(_) => Action::Next,
                None if late => Action::Fail(format!("{selector:?} did not appear")),
                None => Action::Wait,
            },
            Step::Expect(selector, false) => match find(scene, selector) {
                None => Action::Next,
                Some(_) if late => Action::Fail(format!("{selector:?} is still on screen")),
                Some(_) => Action::Wait,
            },
            Step::Click(selector, count) => match find(scene, selector) {
                Some((id, point)) => Action::Click(id, point, *count),
                None if late => Action::Fail(format!("nothing to click for {selector:?}")),
                None => Action::Wait,
            },
            Step::Hover(selector) => match find(scene, selector) {
                Some((_, point)) => Action::Hover(point),
                None if late => Action::Fail(format!("nothing to hover for {selector:?}")),
                None => Action::Wait,
            },
            Step::Type(text, into) => match into {
                Some(selector) => match find(scene, selector) {
                    Some((id, _)) => Action::Type(Some(id), text.clone()),
                    None if late => Action::Fail(format!("no field {selector:?}")),
                    None => Action::Wait,
                },
                None => Action::Type(None, text.clone()),
            },
            Step::Submit(selector) => match find(scene, selector) {
                Some((id, _)) => Action::Submit(id),
                None if late => Action::Fail(format!("no field {selector:?}")),
                None => Action::Wait,
            },
            Step::Key(key) => Action::Key(key.clone()),
        };
        if matches!(
            action,
            Action::Next
                | Action::Click(..)
                | Action::Hover(..)
                | Action::Type(..)
                | Action::Submit(..)
                | Action::Key(..)
        ) {
            self.index += 1;
            self.started = None;
        }
        if let Action::Fail(reason) = &action {
            FAILED.store(true, Ordering::SeqCst);
            return Action::Fail(format!(
                "step {} {step:?}: {reason}\n  on screen: {}",
                self.index + 1,
                visible_texts(scene).join(" | ")
            ));
        }
        action
    }

    #[must_use]
    pub fn steps(&self) -> usize {
        self.steps.len()
    }
}

/// The texts on screen (at most 60, each cut to 40 characters, icons
/// left out), top to bottom, to explain a failure. Hidden ones are marked.
fn visible_texts(scene: &Scene) -> Vec<String> {
    let mut texts: Vec<(f32, f32, String)> = scene
        .nodes()
        .filter(|node| {
            !node.is_synthetic() && matches!(node.kind, NodeKind::Text | NodeKind::Button)
        })
        .filter_map(|node| {
            let text = node.props().text(PropKey::Text)?;
            let words = text.chars().filter(|c| c.is_alphanumeric()).count();
            if words < 2 {
                return None;
            }
            let origin = scene.window_origin(node.id);
            let mut shown: String = text.chars().take(40).collect();
            if !scene.is_effectively_visible(node.id) {
                shown.insert_str(0, "(hidden) ");
            }
            Some((origin.y, origin.x, shown))
        })
        .collect();
    texts.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    texts
        .into_iter()
        .take(60)
        .map(|(_, _, text)| text)
        .collect()
}

/// The visible element a selector names and the window point at its center.
#[must_use]
pub fn find(scene: &Scene, selector: &str) -> Option<(NodeId, Point)> {
    let (key, wanted) = match selector.chars().next() {
        Some('#') => (Some(PropKey::TestId), &selector[1..]),
        Some('@') => (Some(PropKey::AccessibilityLabel), &selector[1..]),
        _ => (None, selector),
    };
    let mut best: Option<(NodeId, Point, f32)> = None;
    for node in scene.nodes() {
        if node.is_synthetic() || !scene.is_effectively_visible(node.id) {
            continue;
        }
        let props = node.props();
        let matches = match key {
            Some(key) => props.text(key) == Some(wanted),
            None => {
                matches!(
                    node.kind,
                    NodeKind::Text | NodeKind::Button | NodeKind::Input
                ) && props
                    .text(PropKey::Text)
                    .is_some_and(|text| text.contains(wanted))
            }
        };
        if !matches || node.frame.width <= 0.0 || node.frame.height <= 0.0 {
            continue;
        }
        let origin = scene.window_origin(node.id);
        let center = Point::new(
            origin.x + node.frame.width / 2.0,
            origin.y + node.frame.height / 2.0,
        );
        // The smallest match: the most specific element.
        let area = node.frame.width * node.frame.height;
        if best.is_none_or(|(_, _, smallest)| area < smallest) {
            best = Some((node.id, center, area));
        }
    }
    best.map(|(id, point, _)| (id, point))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_parse_their_steps_and_refuse_unknown_ones() {
        let driver = Driver::parse(
            r##"[{"wait": "#composer"}, {"click": "general"}, {"type": "hi", "into": "@Message"}, {"key": "Ctrl+K"}, {"expectNot": "Loading"}, {"sleep": 100, "timeout": 5}]"##,
        )
        .unwrap();
        assert_eq!(driver.steps(), 6);
        assert_eq!(
            driver.steps[2].0,
            Step::Type("hi".into(), Some("@Message".into()))
        );
        assert_eq!(driver.steps[5], (Step::Sleep(100), 5));
        assert!(Driver::parse(r#"[{"fly": "away"}]"#).is_err());
    }
}
