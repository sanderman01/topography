//! Hydraulic geometry: channel width and depth from discharge, with the spec's starting values
//! w = 3·Q^0.5 and d = 0.3·Q^0.4 (Q in m³/s), depth capped at 2 m.

use super::CHANNEL_CAP_M;

pub(crate) struct Geometry {
    /// m³/s per km² at mean rain: a game setting (`m3s_per_km2` in water-v3.toml, R5), tuned
    /// so the largest river is about 30 m wide.
    pub m3s_per_km2: f64,
}

impl Geometry {
    pub fn new(m3s_per_km2: f64) -> Self {
        Self { m3s_per_km2 }
    }
    pub fn width_m(&self, q_km2: f64) -> f64 {
        3.0 * (q_km2 * self.m3s_per_km2).sqrt()
    }
    pub fn depth_m(&self, q_km2: f64) -> f32 {
        ((0.3 * (q_km2 * self.m3s_per_km2).powf(0.4)) as f32).min(CHANNEL_CAP_M)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_game_factor_gives_the_widths_of_ruling_r5() {
        let g = Geometry::new(0.56);
        assert!(
            (g.width_m(177.0) - 29.9).abs() < 0.2,
            "the largest river is about 30 m wide"
        );
        assert!(
            (g.depth_m(177.0) - 1.89).abs() < 0.02,
            "and stays under the 2 m cap"
        );
        assert!((g.width_m(100.0) - 22.4).abs() < 0.2, "the canyon trunk");
        assert!((g.width_m(1.0) - 2.24).abs() < 0.02);
        assert!((g.width_m(0.05) - 0.50).abs() < 0.01);
        assert!((g.depth_m(0.05) - 0.072).abs() < 0.003);
        assert_eq!(Geometry::new(10.0).depth_m(177.0), 2.0, "capped");
    }
}
