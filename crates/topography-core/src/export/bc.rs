//! BC4 and BC5 (two BC4 blocks) encoders and decoders for baked terrain layers.
//!
//! The encoder always writes the 8-value mode (red0 > red1): endpoints from the block range,
//! a small endpoint search, then least-squares refinement. It is deterministic.

/// Index code of palette step s (s = 0 is red0, s = 7 is red1).
const CODE: [u64; 8] = [0, 2, 3, 4, 5, 6, 7, 1];

/// Squared error of `v` (0..=255 units) against endpoints r0 > r1, filling each texel's step.
fn fit(v: &[f32; 16], r0: i32, r1: i32, steps: &mut [u8; 16]) -> f32 {
    let (a, b) = (r0 as f32, r1 as f32);
    let span = (a - b) / 7.0;
    let mut sse = 0.0;
    for (s, &x) in steps.iter_mut().zip(v) {
        let t = ((a - x) / span).round().clamp(0.0, 7.0);
        *s = t as u8;
        let d = x - ((7.0 - t) * a + t * b) / 7.0;
        sse += d * d;
    }
    sse
}

/// One BC4 block from 16 values in [0, 1], row-major.
pub(super) fn bc4_block(values: &[f32; 16]) -> [u8; 8] {
    let v = values.map(|x| x.clamp(0.0, 1.0) * 255.0);
    let (lo, hi) = v
        .iter()
        .fold((f32::MAX, f32::MIN), |(l, h), &x| (l.min(x), h.max(x)));
    let mut best = (f32::MAX, 1, 0, [0u8; 16]);
    let mut steps = [0u8; 16];
    let mut try_pair = |r0: i32, r1: i32, best: &mut (f32, i32, i32, [u8; 16])| {
        if r0 > r1 && (0..=255).contains(&r0) && (0..=255).contains(&r1) {
            let sse = fit(&v, r0, r1, &mut steps);
            if sse < best.0 {
                *best = (sse, r0, r1, steps);
            }
        }
    };
    let (h, l) = (hi.round() as i32, lo.round() as i32);
    for d0 in -1..=1 {
        for d1 in -1..=1 {
            try_pair(h + d0, l + d1, &mut best);
        }
    }
    // Least squares for the endpoints given the steps, then their neighbours.
    for _ in 0..3 {
        let (mut aa, mut ab, mut bb, mut av, mut bv) = (0f32, 0f32, 0f32, 0f32, 0f32);
        for (&s, &x) in best.3.iter().zip(&v) {
            let b = f32::from(s) / 7.0;
            let a = 1.0 - b;
            aa += a * a;
            ab += a * b;
            bb += b * b;
            av += a * x;
            bv += b * x;
        }
        let det = aa * bb - ab * ab;
        if det.abs() < 1e-6 * (aa * bb).max(1e-12) {
            break;
        }
        let r0 = ((bb * av - ab * bv) / det).round() as i32;
        let r1 = ((aa * bv - ab * av) / det).round() as i32;
        let before = (best.1, best.2);
        for d0 in -1..=1 {
            for d1 in -1..=1 {
                try_pair(r0 + d0, r1 + d1, &mut best);
            }
        }
        if (best.1, best.2) == before {
            break;
        }
    }
    let (_, r0, r1, steps) = best;
    let bits = steps
        .iter()
        .enumerate()
        .fold(0u64, |acc, (i, &s)| acc | CODE[usize::from(s)] << (3 * i));
    let mut out = [0u8; 8];
    out[0] = r0 as u8;
    out[1] = r1 as u8;
    out[2..].copy_from_slice(&bits.to_le_bytes()[..6]);
    out
}

/// The 16 values in [0, 1] of a BC4 block (both modes), as a GPU decodes them.
pub(super) fn bc4_decode_block(block: &[u8; 8]) -> [f32; 16] {
    let (r0, r1) = (f32::from(block[0]), f32::from(block[1]));
    let mut palette = [r0, r1, 0.0, 0.0, 0.0, 0.0, 0.0, 255.0];
    if block[0] > block[1] {
        for k in 1..7 {
            palette[k + 1] = ((7 - k) as f32 * r0 + k as f32 * r1) / 7.0;
        }
    } else {
        for k in 1..5 {
            palette[k + 1] = ((5 - k) as f32 * r0 + k as f32 * r1) / 5.0;
        }
        palette[6] = 0.0;
    }
    let mut bytes = [0u8; 8];
    bytes[..6].copy_from_slice(&block[2..]);
    let bits = u64::from_le_bytes(bytes);
    std::array::from_fn(|i| palette[((bits >> (3 * i)) & 7) as usize] / 255.0)
}

/// The 4×4 block at block (bx, by) of a row-major image of width `w`.
fn gather(values: &[f32], w: usize, bx: usize, by: usize) -> [f32; 16] {
    std::array::from_fn(|i| values[(4 * by + i / 4) * w + 4 * bx + i % 4])
}

fn scatter(out: &mut [f32], w: usize, bx: usize, by: usize, block: &[f32; 16]) {
    for (i, &v) in block.iter().enumerate() {
        out[(4 * by + i / 4) * w + 4 * bx + i % 4] = v;
    }
}

/// BC4 blocks (row-major, 8 bytes each) of a `w` × `h` image; both multiples of 4.
pub(super) fn bc4(values: &[f32], w: usize, h: usize) -> Vec<u8> {
    debug_assert!(w % 4 == 0 && h % 4 == 0 && values.len() == w * h);
    let mut out = Vec::with_capacity(w * h / 2);
    for by in 0..h / 4 {
        for bx in 0..w / 4 {
            out.extend(bc4_block(&gather(values, w, bx, by)));
        }
    }
    out
}

/// BC5 blocks (red BC4 block, then green, 16 bytes each) of a `w` × `h` image.
pub(super) fn bc5(red: &[f32], green: &[f32], w: usize, h: usize) -> Vec<u8> {
    debug_assert!(w % 4 == 0 && h % 4 == 0 && red.len() == w * h && green.len() == w * h);
    let mut out = Vec::with_capacity(w * h);
    for by in 0..h / 4 {
        for bx in 0..w / 4 {
            out.extend(bc4_block(&gather(red, w, bx, by)));
            out.extend(bc4_block(&gather(green, w, bx, by)));
        }
    }
    out
}

/// Decodes `channels` interleaved BC4 blocks per 4×4 block (1 for BC4, 2 for BC5) into
/// one row-major plane per channel.
pub(super) fn decode(blocks: &[u8], w: usize, h: usize, channels: usize) -> Vec<Vec<f32>> {
    let mut out = vec![vec![0f32; w * h]; channels];
    let mut chunks = blocks.chunks_exact(8);
    for by in 0..h / 4 {
        for bx in 0..w / 4 {
            for plane in &mut out {
                let block: &[u8; 8] = chunks
                    .next()
                    .and_then(|c| c.try_into().ok())
                    .expect("block count matches the image size");
                scatter(plane, w, bx, by, &bc4_decode_block(block));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip_max(values: &[f32; 16]) -> f32 {
        let d = bc4_decode_block(&bc4_block(values));
        values
            .iter()
            .zip(d)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0, f32::max)
    }

    #[test]
    fn constant_blocks_are_near_exact() {
        for k in 0..=100 {
            let v = k as f32 / 100.0;
            // 8-value mode between adjacent codes: steps of 1/7 code.
            assert!(roundtrip_max(&[v; 16]) <= 0.5 / 7.0 / 255.0 + 1e-6, "{v}");
        }
        assert_eq!(roundtrip_max(&[0.0; 16]), 0.0);
        assert_eq!(roundtrip_max(&[1.0; 16]), 0.0);
    }

    #[test]
    fn ramps_and_noise_stay_within_their_palette_step() {
        let ramp: [f32; 16] = std::array::from_fn(|i| 0.2 + 0.3 * i as f32 / 15.0);
        // Range 0.3: half a palette step is 0.3 / 14, plus endpoint rounding.
        assert!(roundtrip_max(&ramp) <= 0.3 / 14.0 + 1.0 / 255.0);
        // Noise: least squares trades the maximum for the mean, so allow a whole step.
        let mut seed = 12345u32;
        for _ in 0..1000 {
            let noise: [f32; 16] = std::array::from_fn(|_| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                (seed >> 8) as f32 / (1u32 << 24) as f32
            });
            let (lo, hi) = noise
                .iter()
                .fold((1f32, 0f32), |(l, h), &x| (l.min(x), h.max(x)));
            assert!(roundtrip_max(&noise) <= (hi - lo) / 7.0 + 1.0 / 255.0);
            let d = bc4_decode_block(&bc4_block(&noise));
            let mse = noise
                .iter()
                .zip(d)
                .map(|(a, b)| (a - b).powi(2))
                .sum::<f32>()
                / 16.0;
            assert!(mse.sqrt() <= (hi - lo) / 14.0);
        }
    }

    #[test]
    fn encoding_is_deterministic_and_8_value() {
        let v: [f32; 16] = std::array::from_fn(|i| (i as f32 * 0.37).sin() * 0.5 + 0.5);
        let a = bc4_block(&v);
        assert_eq!(a, bc4_block(&v));
        assert!(a[0] > a[1]);
    }

    #[test]
    fn bc5_images_round_trip_per_channel() {
        let (w, h) = (8, 4);
        let red: Vec<f32> = (0..w * h).map(|i| (i % w) as f32 / 8.0).collect();
        let green: Vec<f32> = (0..w * h).map(|i| (i / w) as f32 / 4.0).collect();
        let blocks = bc5(&red, &green, w, h);
        assert_eq!(blocks.len(), 32);
        let planes = decode(&blocks, w, h, 2);
        // Within half a palette step of each block's range (red 0.375, green 0.75).
        for (plane, src, range) in [(&planes[0], &red, 0.375), (&planes[1], &green, 0.75)] {
            for (a, b) in plane.iter().zip(src.iter()) {
                assert!((a - b).abs() <= range / 14.0 + 1.0 / 255.0, "{a} {b}");
            }
        }
        let single = bc4(&red, w, h);
        assert_eq!(decode(&single, w, h, 1)[0], planes[0]);
    }

    #[test]
    fn six_value_blocks_decode_with_their_extremes() {
        // red0 <= red1: codes 6 and 7 are 0 and 1.
        let mut block = [10u8, 20, 0, 0, 0, 0, 0, 0];
        let bits: u64 = 6 | (7 << 3) | (2 << 6);
        block[2..].copy_from_slice(&bits.to_le_bytes()[..6]);
        let d = bc4_decode_block(&block);
        assert_eq!((d[0], d[1]), (0.0, 1.0));
        assert!((d[2] - 12.0 / 255.0).abs() < 1e-6);
    }
}
