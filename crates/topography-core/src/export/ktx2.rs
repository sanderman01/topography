//! Minimal KTX 2.0 writer: one 2D image, one mip level, no supercompression, BC4/BC5 UNORM.

/// Block-compressed formats written by the exporter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Format {
    Bc4Unorm,
    Bc5Unorm,
}

impl Format {
    pub fn vk_format(self) -> u32 {
        match self {
            Self::Bc4Unorm => 139,
            Self::Bc5Unorm => 141,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Bc4Unorm => "VK_FORMAT_BC4_UNORM_BLOCK",
            Self::Bc5Unorm => "VK_FORMAT_BC5_UNORM_BLOCK",
        }
    }

    pub fn block_bytes(self) -> usize {
        match self {
            Self::Bc4Unorm => 8,
            Self::Bc5Unorm => 16,
        }
    }

    /// KHR_DF_MODEL_BC4 / KHR_DF_MODEL_BC5.
    fn color_model(self) -> u8 {
        match self {
            Self::Bc4Unorm => 131,
            Self::Bc5Unorm => 132,
        }
    }
}

const IDENTIFIER: [u8; 12] = [
    0xAB, 0x4B, 0x54, 0x58, 0x20, 0x32, 0x30, 0xBB, 0x0D, 0x0A, 0x1A, 0x0A,
];
/// Header (80 bytes) plus one level index entry (24 bytes).
const DFD_OFFSET: usize = 80 + 24;

fn u32s(out: &mut Vec<u8>, values: &[u32]) {
    for v in values {
        out.extend(v.to_le_bytes());
    }
}

/// Data format descriptor: total size, then one basic block with a 64-bit sample per BC4
/// channel (BC5: red at bit 0, green at bit 64).
fn dfd(format: Format) -> Vec<u8> {
    let samples = format.block_bytes() / 8;
    let block_size = 24 + 16 * samples as u32;
    let mut out = Vec::new();
    u32s(&mut out, &[4 + block_size, 0, 2 | (block_size << 16)]);
    // Model, primaries BT.709, transfer linear, flags straight alpha.
    out.extend([format.color_model(), 1, 1, 0]);
    // Texel block 4×4×1×1 (dimension − 1).
    out.extend([3, 3, 0, 0]);
    u32s(&mut out, &[format.block_bytes() as u32, 0]);
    for s in 0..samples as u32 {
        // Bit offset, bit length − 1, channel id (BC4 data, BC5 red / green); no qualifiers.
        u32s(
            &mut out,
            &[(64 * s) | (63 << 16) | (s << 24), 0, 0, u32::MAX],
        );
    }
    out
}

/// Key/value entries, sorted by key; each value NUL-terminated and padded to 4 bytes.
fn kvd(entries: &[(&str, &str)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (key, value) in entries {
        let len = key.len() + value.len() + 2;
        u32s(&mut out, &[len as u32]);
        out.extend(key.as_bytes());
        out.push(0);
        out.extend(value.as_bytes());
        out.push(0);
        out.resize(out.len().next_multiple_of(4), 0);
    }
    out
}

/// A KTX2 file of `width` × `height` texels (multiples of 4) holding `blocks`.
pub(super) fn write(format: Format, width: usize, height: usize, blocks: &[u8]) -> Vec<u8> {
    debug_assert!(width % 4 == 0 && height % 4 == 0);
    debug_assert_eq!(blocks.len(), width * height / 16 * format.block_bytes());
    let dfd = dfd(format);
    let writer = format!("topography {}", env!("CARGO_PKG_VERSION"));
    let kvd = kvd(&[("KTXorientation", "rd"), ("KTXwriter", &writer)]);
    let kvd_offset = DFD_OFFSET + dfd.len();
    // Level data is aligned to lcm(block size, 4).
    let data_offset = (kvd_offset + kvd.len()).next_multiple_of(format.block_bytes());
    let mut out = Vec::with_capacity(data_offset + blocks.len());
    out.extend(IDENTIFIER);
    // vkFormat, typeSize, width, height, depth, layers, faces, levels, supercompression.
    u32s(
        &mut out,
        &[
            format.vk_format(),
            1,
            width as u32,
            height as u32,
            0,
            0,
            1,
            1,
            0,
        ],
    );
    u32s(
        &mut out,
        &[
            DFD_OFFSET as u32,
            dfd.len() as u32,
            kvd_offset as u32,
            kvd.len() as u32,
        ],
    );
    // No supercompression global data.
    out.extend([0u8; 16]);
    for v in [data_offset, blocks.len(), blocks.len()] {
        out.extend((v as u64).to_le_bytes());
    }
    out.extend(dfd);
    out.extend(kvd);
    out.resize(data_offset, 0);
    out.extend(blocks);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u32_at(b: &[u8], at: usize) -> u32 {
        u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
    }

    fn u64_at(b: &[u8], at: usize) -> u64 {
        u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
    }

    #[test]
    fn header_fields_follow_the_spec_layout() {
        let blocks = vec![7u8; 16 * 4];
        let f = write(Format::Bc5Unorm, 8, 8, &blocks);
        assert_eq!(f[..12], IDENTIFIER);
        let fields: Vec<u32> = (0..9).map(|i| u32_at(&f, 12 + 4 * i)).collect();
        assert_eq!(fields, [141, 1, 8, 8, 0, 0, 1, 1, 0]);
        let (dfd_off, dfd_len) = (u32_at(&f, 48) as usize, u32_at(&f, 52) as usize);
        let (kvd_off, kvd_len) = (u32_at(&f, 56) as usize, u32_at(&f, 60) as usize);
        assert_eq!((dfd_off, dfd_len), (104, 4 + 24 + 32));
        assert_eq!(kvd_off, dfd_off + dfd_len);
        assert_eq!((u64_at(&f, 64), u64_at(&f, 72)), (0, 0));
        let (off, len, raw) = (u64_at(&f, 80), u64_at(&f, 88), u64_at(&f, 96));
        assert_eq!((len, raw), (64, 64));
        assert_eq!(off % 16, 0);
        assert!(off as usize >= kvd_off + kvd_len);
        assert_eq!(&f[off as usize..], &blocks[..]);
        // DFD: total size, vendor/type 0, version 2 and block size 56.
        assert_eq!(u32_at(&f, dfd_off), dfd_len as u32);
        assert_eq!(u32_at(&f, dfd_off + 4), 0);
        assert_eq!(u32_at(&f, dfd_off + 8), 2 | (56 << 16));
        assert_eq!(f[dfd_off + 12..dfd_off + 20], [132, 1, 1, 0, 3, 3, 0, 0]);
        assert_eq!(u32_at(&f, dfd_off + 20), 16);
        // Green sample: bit offset 64, length 64, channel 1.
        assert_eq!(u32_at(&f, dfd_off + 44), 64 | (63 << 16) | (1 << 24));
        assert_eq!(u32_at(&f, dfd_off + 56), u32::MAX);
        // First KVD entry: length then "KTXorientation\0rd\0".
        assert_eq!(u32_at(&f, kvd_off), 18);
        assert_eq!(&f[kvd_off + 4..kvd_off + 22], b"KTXorientation\0rd\0");
        assert_eq!(kvd_len % 4, 0);
    }

    #[test]
    fn ktx2_reader_parses_both_formats() {
        use ::ktx2::{ColorModel, Reader, TransferFunction, dfd};
        for (format, vk, model, samples) in [
            (
                Format::Bc4Unorm,
                ::ktx2::Format::BC4_UNORM_BLOCK,
                ColorModel::BC4,
                1,
            ),
            (
                Format::Bc5Unorm,
                ::ktx2::Format::BC5_UNORM_BLOCK,
                ColorModel::BC5,
                2,
            ),
        ] {
            let blocks: Vec<u8> = (0..format.block_bytes() * 6).map(|i| i as u8).collect();
            let file = write(format, 12, 8, &blocks);
            let r = Reader::new(&file[..]).expect("valid KTX2");
            let h = r.header();
            assert_eq!(h.format, Some(vk));
            assert_eq!(h.format.map(|f| f.value()), Some(format.vk_format()));
            assert_eq!((h.pixel_width, h.pixel_height, h.level_count), (12, 8, 1));
            assert!(h.supercompression_scheme.is_none());
            assert_eq!(r.levels().next().unwrap().data, &blocks[..]);
            let basic = r.basic_dfd().expect("basic DFD");
            assert_eq!(basic.color_model, Some(model));
            assert_eq!(basic.transfer_function, Some(TransferFunction::Linear));
            assert_eq!(basic.texel_block_dimensions.map(|d| d.get()), [4, 4, 1, 1]);
            assert_eq!(basic.sample_information.len(), samples);
            assert_eq!(basic.sample_information[samples - 1].bit_length.get(), 64);
            // The reference DFD the crate generates for this vkFormat matches ours byte for byte.
            let (reference, type_size) = dfd::Basic::from_format(vk).unwrap();
            assert_eq!(type_size, 1);
            assert_eq!(dfd::Block::Basic(reference).to_vec(), dfd(format)[4..]);
            assert_eq!(r.writer().map(|w| w.starts_with("topography")), Some(true));
            let keys: Vec<_> = r.key_value_data().map(|(k, _)| k).collect();
            assert_eq!(keys, ["KTXorientation", "KTXwriter"]);
        }
    }
}
