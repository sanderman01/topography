//! The 128 m moisture field from the 32 m Smith–Barstad stage and each cell's rain in integer
//! Q units, so discharge sums exactly across tiles.

use crate::acquisition::{AcquisitionError, Result, ensure, io_context};
use serde_json::{Map, Value};
use std::{fs, path::Path};

/// Q units per km² at mean rain: Q is a u64 count of 1e-12 km² (1e-6 m²).
pub(crate) const Q_PER_KM2: f64 = 1e12;

pub(crate) struct Rain {
    pub(crate) w: usize,
    pub(crate) h: usize,
    pub(crate) spacing_m: f64,
    pub(crate) nw_m: [f64; 2],
    pub(crate) values: Vec<f32>,
    /// Discharge weight per moisture unit: scale_mm_h / mean_land_mm_h (as v2).
    pub(crate) weight: f64,
    pub mean_land_mm_h: f64,
    /// The grid.json `moisture` entry, passed on to the outputs.
    pub entry: Map<String, Value>,
}

impl Rain {
    /// From a grid directory whose grid.json has a `moisture` entry (file, size, spacing_m,
    /// nw_m, scale_mm_h) and `discharge.mean_land_mm_h`.
    pub fn read(dir: &Path, manifest: &Map<String, Value>) -> Result<Self> {
        let entry = manifest
            .get("moisture")
            .and_then(Value::as_object)
            .cloned()
            .ok_or_else(|| AcquisitionError::Invalid("grid.json lacks a moisture entry".into()))?;
        let [w, h]: [usize; 2] = serde_json::from_value(entry["size"].clone())
            .map_err(|_| AcquisitionError::Invalid("moisture size must be [w, h]".into()))?;
        let nw_m: [f64; 2] = serde_json::from_value(entry["nw_m"].clone())
            .map_err(|_| AcquisitionError::Invalid("moisture nw_m must be [x, y]".into()))?;
        let spacing_m = entry["spacing_m"].as_f64().unwrap_or(f64::NAN);
        let scale = entry["scale_mm_h"].as_f64().unwrap_or(f64::NAN);
        let mean = manifest
            .get("discharge")
            .and_then(|d| d["mean_land_mm_h"].as_f64())
            .unwrap_or(f64::NAN);
        ensure(
            spacing_m > 0.0 && scale > 0.0 && mean > 0.0,
            "moisture needs spacing_m, scale_mm_h and discharge.mean_land_mm_h",
        )?;
        let file = entry["file"].as_str().unwrap_or("moisture.f32");
        let bytes = io_context(format!("read {file}"), fs::read(dir.join(file)))?;
        ensure(
            bytes.len() == w * h * 4,
            "moisture.f32 does not match its size",
        )?;
        let values = bytes
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
            .collect();
        Ok(Self {
            w,
            h,
            spacing_m,
            nw_m,
            values,
            weight: scale / mean,
            mean_land_mm_h: mean,
            entry,
        })
    }

    /// Moisture 1 and weight 1 everywhere (tests).
    pub fn uniform(mean_land_mm_h: f64) -> Self {
        Self {
            w: 1,
            h: 1,
            spacing_m: 1e9,
            nw_m: [-5e8, 5e8],
            values: vec![1.0],
            weight: 1.0,
            mean_land_mm_h,
            entry: Map::new(),
        }
    }

    /// Bilinear between cell centres at a point of the layout plane (x east, y north), as v2.
    pub fn at(&self, x: f64, y: f64) -> f64 {
        let fx = ((x - self.nw_m[0]) / self.spacing_m - 0.5).clamp(0.0, (self.w - 1) as f64);
        let fy = ((self.nw_m[1] - y) / self.spacing_m - 0.5).clamp(0.0, (self.h - 1) as f64);
        let (x0, y0) = (fx as usize, fy as usize);
        let (x1, y1) = ((x0 + 1).min(self.w - 1), (y0 + 1).min(self.h - 1));
        let (ax, ay) = (fx - x0 as f64, fy - y0 as f64);
        let p = |x: usize, y: usize| f64::from(self.values[y * self.w + x]);
        let top = p(x0, y0) * (1.0 - ax) + p(x1, y0) * ax;
        let bottom = p(x0, y1) * (1.0 - ax) + p(x1, y1) * ax;
        top * (1.0 - ay) + bottom * ay
    }

    /// A cell's rain in Q units (1e-12 km² at mean rain) at its centre (x, y).
    pub fn units(&self, x: f64, y: f64, cell_km2: f64) -> u64 {
        (self.at(x, y) * self.weight * cell_km2 * Q_PER_KM2).round() as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rain_is_bilinear_and_in_integer_units() {
        let r = Rain {
            w: 2,
            h: 1,
            spacing_m: 128.0,
            nw_m: [0.0, 128.0],
            values: vec![0.0, 1.0],
            weight: 2.0,
            mean_land_mm_h: 2.0,
            entry: Default::default(),
        };
        assert_eq!(r.at(64.0, 64.0), 0.0);
        assert_eq!(r.at(128.0, 64.0), 0.5);
        assert_eq!(r.at(500.0, 64.0), 1.0, "clamped past the last centre");
        // 1 m² cell at moisture 0.5, weight 2: 1e-6 km² = 1e6 units.
        assert_eq!(r.units(128.0, 64.0, 1e-6), 1_000_000);
    }
}
