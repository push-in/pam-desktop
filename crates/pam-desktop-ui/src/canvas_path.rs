//! Bounded absolute M/L/C/Q/Z paths shared by native drawing backends.

use crate::backend::DrawPathSegment;
use crate::geometry::Point;

pub fn parse(spec: &str) -> Option<Vec<DrawPathSegment>> {
    if spec.is_empty() || spec.len() > 4096 || !spec.is_ascii() {
        return None;
    }
    let mut expanded = String::with_capacity(spec.len() + 64);
    for character in spec.chars() {
        if "MLCQZ".contains(character) {
            expanded.push(' ');
            expanded.push(character);
            expanded.push(' ');
        } else if character.is_ascii_digit() || " .+-eE\t\n\r".contains(character) {
            expanded.push(character);
        } else {
            return None;
        }
    }
    let mut tokens = expanded.split_whitespace();
    let mut result = Vec::new();
    let mut current = Point::new(0.0, 0.0);
    let mut start = current;
    let number = |tokens: &mut std::str::SplitWhitespace<'_>| -> Option<f32> {
        let value: f32 = tokens.next()?.parse().ok()?;
        (value.is_finite() && value.abs() <= 10_000_000.0).then_some(value)
    };
    while let Some(verb) = tokens.next() {
        if result.is_empty() && verb != "M" {
            return None;
        }
        let segment = match verb {
            "M" | "L" => {
                let point = Point::new(number(&mut tokens)?, number(&mut tokens)?);
                current = point;
                if verb == "M" {
                    start = point;
                    DrawPathSegment::Move(point)
                } else {
                    DrawPathSegment::Line(point)
                }
            }
            "C" => {
                let control1 = Point::new(number(&mut tokens)?, number(&mut tokens)?);
                let control2 = Point::new(number(&mut tokens)?, number(&mut tokens)?);
                let to = Point::new(number(&mut tokens)?, number(&mut tokens)?);
                current = to;
                DrawPathSegment::Cubic {
                    control1,
                    control2,
                    to,
                }
            }
            "Q" => {
                let control = Point::new(number(&mut tokens)?, number(&mut tokens)?);
                let to = Point::new(number(&mut tokens)?, number(&mut tokens)?);
                let control1 = Point::new(
                    current.x + (control.x - current.x) * 2.0 / 3.0,
                    current.y + (control.y - current.y) * 2.0 / 3.0,
                );
                let control2 = Point::new(
                    to.x + (control.x - to.x) * 2.0 / 3.0,
                    to.y + (control.y - to.y) * 2.0 / 3.0,
                );
                current = to;
                DrawPathSegment::Cubic {
                    control1,
                    control2,
                    to,
                }
            }
            "Z" => {
                current = start;
                DrawPathSegment::Close
            }
            _ => return None,
        };
        result.push(segment);
    }
    (!result.is_empty()).then_some(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn decodes_curves_and_rejects_malformed_or_unbounded_paths() {
        assert_eq!(
            parse("M0 0 Q3 3 6 0 Z"),
            Some(vec![
                DrawPathSegment::Move(Point::new(0.0, 0.0)),
                DrawPathSegment::Cubic {
                    control1: Point::new(2.0, 2.0),
                    control2: Point::new(4.0, 2.0),
                    to: Point::new(6.0, 0.0)
                },
                DrawPathSegment::Close,
            ])
        );
        for invalid in [
            "",
            "L1 2",
            "M1",
            "M0 0 C1 2",
            "M0 0 LNaN 1",
            "M1e99 2",
            "M0 0 A1 2",
        ] {
            assert!(parse(invalid).is_none(), "{invalid}");
        }
    }
}
