//! Original RGB monitor profile for a PDF/A output intent.
//!
//! The numbers are sRGB-like primaries written as ICC s15Fixed16 values.
//! This is not a copy of a vendor profile. The D50 illuminant uses the
//! constants stored by the ICC specification rather than a fresh rounding
//! of 0.9642.

/// Builds a 468-byte ICC v2 RGB display profile.
pub(crate) fn srgb_profile() -> Vec<u8> {
    let mut profile = vec![0u8; 468];
    write_u32(&mut profile, 8, 0x02100000);
    write_tag(&mut profile, 12, b"mntr");
    write_tag(&mut profile, 16, b"RGB ");
    write_tag(&mut profile, 20, b"XYZ ");
    write_u16(&mut profile, 24, 2026);
    write_u16(&mut profile, 26, 1);
    write_u16(&mut profile, 28, 1);
    write_tag(&mut profile, 36, b"acsp");
    write_s15(&mut profile, 68, 63190);
    write_s15(&mut profile, 72, 65536);
    write_s15(&mut profile, 76, 54061);

    write_u32(&mut profile, 128, 9);
    let records = [
        (*b"desc", 240u32, 108u32),
        (*b"rXYZ", 348, 20),
        (*b"gXYZ", 368, 20),
        (*b"bXYZ", 388, 20),
        (*b"rTRC", 408, 16),
        (*b"gTRC", 408, 16),
        (*b"bTRC", 408, 16),
        (*b"wtpt", 424, 20),
        (*b"cprt", 444, 24),
    ];
    for (index, (tag, offset, size)) in records.iter().enumerate() {
        let at = 132 + index * 12;
        write_tag(&mut profile, at, tag);
        write_u32(&mut profile, at + 4, *offset);
        write_u32(&mut profile, at + 8, *size);
    }

    write_description(&mut profile, 240);
    write_xyz(&mut profile, 348, [0.436066, 0.222488, 0.013916]);
    write_xyz(&mut profile, 368, [0.385147, 0.716873, 0.097076]);
    write_xyz(&mut profile, 388, [0.143066, 0.060608, 0.714096]);
    write_curve(&mut profile, 408);
    write_xyz_raw(&mut profile, 424, [63190, 65536, 54061]);
    write_copyright(&mut profile, 444);
    let profile_len = profile.len() as u32;
    write_u32(&mut profile, 0, profile_len);

    debug_assert_eq!(profile.len(), 468);
    debug_assert_eq!(&profile[36..40], b"acsp");
    profile
}

fn write_description(profile: &mut [u8], at: usize) {
    write_tag(profile, at, b"desc");
    let text = b"sRGB monitor\0";
    write_u32(profile, at + 8, text.len() as u32);
    profile[at + 12..at + 12 + text.len()].copy_from_slice(text);
    // ASCII ends on a 4-byte boundary at at+28. Unicode language and count
    // stay zero, then the 70-byte scriptcode tail required by ICC.1:1998.
}

fn write_xyz(profile: &mut [u8], at: usize, values: [f64; 3]) {
    write_tag(profile, at, b"XYZ ");
    for (index, value) in values.iter().enumerate() {
        let fixed = (value * 65536.0).round() as i32;
        write_s15(profile, at + 8 + index * 4, fixed);
    }
}

fn write_xyz_raw(profile: &mut [u8], at: usize, values: [i32; 3]) {
    write_tag(profile, at, b"XYZ ");
    for (index, value) in values.iter().enumerate() {
        write_s15(profile, at + 8 + index * 4, *value);
    }
}

fn write_curve(profile: &mut [u8], at: usize) {
    write_tag(profile, at, b"curv");
    write_u32(profile, at + 8, 1);
    write_u16(profile, at + 12, 563);
}

fn write_copyright(profile: &mut [u8], at: usize) {
    write_tag(profile, at, b"text");
    let text = b"Public domain\0";
    profile[at + 8..at + 8 + text.len()].copy_from_slice(text);
}

fn write_tag(profile: &mut [u8], at: usize, tag: &[u8; 4]) {
    profile[at..at + 4].copy_from_slice(tag);
}

fn write_u16(profile: &mut [u8], at: usize, value: u16) {
    profile[at..at + 2].copy_from_slice(&value.to_be_bytes());
}

fn write_u32(profile: &mut [u8], at: usize, value: u32) {
    profile[at..at + 4].copy_from_slice(&value.to_be_bytes());
}

fn write_s15(profile: &mut [u8], at: usize, value: i32) {
    profile[at..at + 4].copy_from_slice(&value.to_be_bytes());
}
