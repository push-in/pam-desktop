//! Explicit fractional tracks. Work and storage are bounded by 64 columns.
#[derive(Debug, Clone)]
pub struct GridTracks(Vec<(f32, f32)>);

/// The track wire is malformed, out of range or exceeds the 64-column bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidGridTracks;

impl std::fmt::Display for InvalidGridTracks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("invalid grid tracks")
    }
}

impl std::error::Error for InvalidGridTracks {}

impl GridTracks {
    pub fn parse(wire: &str) -> Result<Self, InvalidGridTracks> {
        if wire.len() > 8192 {
            return Err(InvalidGridTracks);
        }
        let mut tracks = Vec::new();
        for track in wire.split(';') {
            let (minimum, weight) = track.split_once(',').ok_or(InvalidGridTracks)?;
            let minimum: f32 = minimum.parse().map_err(|_| InvalidGridTracks)?;
            let weight: f32 = weight.parse().map_err(|_| InvalidGridTracks)?;
            if !minimum.is_finite()
                || !weight.is_finite()
                || !(0.0..=1_000_000.0).contains(&minimum)
                || weight <= 0.0
                || weight > 1_000_000.0
                || tracks.len() >= 64
            {
                return Err(InvalidGridTracks);
            }
            tracks.push((minimum, weight));
        }
        if tracks.is_empty() {
            return Err(InvalidGridTracks);
        }
        Ok(Self(tracks))
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
    /// Always false for a parsed value: `parse` rejects an empty track list.
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn widths(&self, available: f32) -> Vec<f32> {
        let mut widths = vec![0.0; self.len()];
        let mut frozen = vec![false; self.len()];
        let mut remaining = available.max(0.0);
        loop {
            let weight: f32 = self
                .0
                .iter()
                .enumerate()
                .filter(|(i, _)| !frozen[*i])
                .map(|(_, t)| t.1)
                .sum();
            if weight <= 0.0 {
                break;
            }
            let unit = remaining.max(0.0) / weight;
            let mut changed = false;
            for (i, (minimum, ratio)) in self.0.iter().enumerate() {
                if !frozen[i] && *minimum > unit * ratio {
                    widths[i] = *minimum;
                    frozen[i] = true;
                    remaining -= minimum;
                    changed = true;
                }
            }
            if !changed {
                for (i, (_, ratio)) in self.0.iter().enumerate() {
                    if !frozen[i] {
                        widths[i] = unit * ratio;
                    }
                }
                break;
            }
        }
        widths
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fractional_tracks_respect_minimum_and_redistribute() {
        let tracks = GridTracks::parse("0,1.8;300,0.9").unwrap();
        let wide = tracks.widths(1200.0);
        assert!((wide[0] - 800.0).abs() < 0.01 && (wide[1] - 400.0).abs() < 0.01);
        assert_eq!(tracks.widths(600.0), vec![300.0, 300.0]);
        assert_eq!(tracks.widths(200.0), vec![0.0, 300.0]);
        for wire in ["", "0,0", "-1,1", "0,NaN", "0,inf", "0,1,2"] {
            assert!(GridTracks::parse(wire).is_err());
        }
        assert!(GridTracks::parse(&vec!["0,1"; 65].join(";")).is_err());
    }
}
