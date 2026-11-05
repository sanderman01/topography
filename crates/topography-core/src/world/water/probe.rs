//! Test-only: windows of the l4v2 raw 1 m surface and hillshade PNGs, for the `#[ignore]`
//! real-terrain probes each task's expert review reads.

use super::geom::{Rect, World};
use super::layers::Layer;
use std::path::Path;

pub(crate) const RAW: &str = "../../data/worlds/fujix-55-l4v2/elevation-raw.f32";
pub(crate) const OUT: &str = "../../scratch/hydrology-v3/probes";
pub(crate) const WORLD: World = World { w: 65536, h: 65536 };

/// A `size`² window of the raw surface centred on (east, north) metres; None only when the
/// file is missing. A window outside the world or an unreadable file panics.
pub(crate) fn window(east: u64, north: u64, size: usize) -> Option<(Rect, Vec<f32>)> {
    if !Path::new(RAW).exists() {
        eprintln!("no data: {RAW}");
        return None;
    }
    let half = size as u64 / 2;
    // Row 0 is the north edge: north metre n is row h - 1 - n.
    let row = WORLD.h.checked_sub(north + 1);
    let x0 = east.checked_sub(half);
    let y0 = row.and_then(|r| r.checked_sub(half));
    let (Some(x0), Some(y0)) = (x0, y0) else {
        panic!("probe window E{east} N{north} size {size} leaves the world");
    };
    assert!(
        x0 + size as u64 <= WORLD.w && y0 + size as u64 <= WORLD.h,
        "probe window E{east} N{north} size {size} leaves the world"
    );
    let r = Rect {
        x0,
        y0,
        w: size,
        h: size,
    };
    let layer =
        Layer::<f32>::open(WORLD, Path::new(RAW)).unwrap_or_else(|e| panic!("open {RAW}: {e}"));
    let z = layer
        .read(r)
        .unwrap_or_else(|e| panic!("read {RAW} window {r:?}: {e}"));
    Some((r, z))
}

/// A hillshade (north-west sun) of `z`, `w` cells wide, with `wet` cells tinted blue, written
/// to scratch/hydrology-v3/probes/<name>.png.
pub(crate) fn shade_png(name: &str, z: &[f32], w: usize, wet: &[bool]) {
    let h = z.len() / w;
    let at = |x: usize, y: usize| f64::from(z[y.min(h - 1) * w + x.min(w - 1)]);
    let mut rgb = Vec::with_capacity(w * h * 3);
    for y in 0..h {
        for x in 0..w {
            let gx = (at(x + 1, y) - at(x.saturating_sub(1), y)) / 2.0;
            let gy = (at(x, y + 1) - at(x, y.saturating_sub(1))) / 2.0;
            let (slope, aspect) = (gx.hypot(gy).atan(), (-gx).atan2(gy));
            let (az, alt) = (315f64.to_radians(), 45f64.to_radians());
            let s = alt.sin() * slope.cos() + alt.cos() * slope.sin() * (az - aspect).cos();
            let g = (s.clamp(0.0, 1.0) * 255.0) as u8;
            if wet[y * w + x] {
                rgb.extend_from_slice(&[g / 3, g / 2, g]);
            } else {
                rgb.extend_from_slice(&[g, g, g]);
            }
        }
    }
    std::fs::create_dir_all(OUT).expect("probe dir");
    let file = std::fs::File::create(Path::new(OUT).join(format!("{name}.png"))).expect("png");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header()
        .expect("png header")
        .write_image_data(&rgb)
        .expect("png data");
    eprintln!("wrote {OUT}/{name}.png");
}
