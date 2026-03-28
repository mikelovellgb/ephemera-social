//! Low-level MP4 atom/box builders for test MP4 construction.
//!
//! Builds individual ISO BMFF boxes as raw bytes for test file generation.

/// Identity matrix in fixed-point 16.16 / 2.30 format for MP4 boxes.
const IDENTITY_MATRIX: [u32; 9] = [0x0001_0000, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000];

/// Write a box header (size + type) and return the buffer.
fn box_start(box_type: &[u8; 4], content_len: usize) -> Vec<u8> {
    let size = (8 + content_len) as u32;
    let mut b = Vec::with_capacity(8 + content_len);
    b.extend_from_slice(&size.to_be_bytes());
    b.extend_from_slice(box_type);
    b
}

pub fn build_ftyp() -> Vec<u8> {
    let brands = b"isom\x00\x00\x02\x00isomiso2avc1mp41";
    let mut b = box_start(b"ftyp", brands.len());
    b.extend_from_slice(brands);
    b
}

pub fn build_moov(w: u16, h: u16, ts: u32, dur: u32, sc: u32) -> Vec<u8> {
    let mvhd = build_mvhd(ts, dur);
    let trak = build_trak(w, h, ts, dur, sc);
    let mut b = box_start(b"moov", mvhd.len() + trak.len());
    b.extend_from_slice(&mvhd);
    b.extend_from_slice(&trak);
    b
}

fn build_mvhd(timescale: u32, duration: u32) -> Vec<u8> {
    let mut b = box_start(b"mvhd", 100);
    b.extend_from_slice(&[0u8; 4]); // Version 0, flags.
    b.extend_from_slice(&[0u8; 8]); // Creation/modification time.
    b.extend_from_slice(&timescale.to_be_bytes());
    b.extend_from_slice(&duration.to_be_bytes());
    b.extend_from_slice(&0x0001_0000u32.to_be_bytes()); // Rate 1.0.
    b.extend_from_slice(&0x0100u16.to_be_bytes()); // Volume 1.0.
    b.extend_from_slice(&[0u8; 10]); // Reserved.
    for m in &IDENTITY_MATRIX {
        b.extend_from_slice(&m.to_be_bytes());
    }
    b.extend_from_slice(&[0u8; 24]); // Pre-defined.
    b.extend_from_slice(&2u32.to_be_bytes()); // Next track ID.
    b
}

fn build_trak(w: u16, h: u16, ts: u32, dur: u32, sc: u32) -> Vec<u8> {
    let tkhd = build_tkhd(w, h, dur);
    let mdia = build_mdia(w, h, ts, dur, sc);
    let mut b = box_start(b"trak", tkhd.len() + mdia.len());
    b.extend_from_slice(&tkhd);
    b.extend_from_slice(&mdia);
    b
}

fn build_tkhd(width: u16, height: u16, duration: u32) -> Vec<u8> {
    let mut b = box_start(b"tkhd", 84);
    b.extend_from_slice(&[0x00, 0x00, 0x00, 0x03]); // Version 0, flags=3.
    b.extend_from_slice(&[0u8; 8]); // Creation/modification time.
    b.extend_from_slice(&1u32.to_be_bytes()); // Track ID.
    b.extend_from_slice(&[0u8; 4]); // Reserved.
    b.extend_from_slice(&duration.to_be_bytes());
    b.extend_from_slice(&[0u8; 8]); // Reserved.
    b.extend_from_slice(&[0u8; 4]); // Layer, alternate group.
    b.extend_from_slice(&[0u8; 4]); // Volume + reserved.
    for m in &IDENTITY_MATRIX {
        b.extend_from_slice(&m.to_be_bytes());
    }
    b.extend_from_slice(&(u32::from(width) << 16).to_be_bytes());
    b.extend_from_slice(&(u32::from(height) << 16).to_be_bytes());
    b
}

fn build_mdia(w: u16, h: u16, ts: u32, dur: u32, sc: u32) -> Vec<u8> {
    let mdhd = build_mdhd(ts, dur);
    let hdlr = build_hdlr_vide();
    let minf = build_minf(w, h, sc);
    let mut b = box_start(b"mdia", mdhd.len() + hdlr.len() + minf.len());
    b.extend_from_slice(&mdhd);
    b.extend_from_slice(&hdlr);
    b.extend_from_slice(&minf);
    b
}

fn build_mdhd(timescale: u32, duration: u32) -> Vec<u8> {
    let mut b = box_start(b"mdhd", 24);
    b.extend_from_slice(&[0u8; 4]); // Version 0, flags.
    b.extend_from_slice(&[0u8; 8]); // Creation/modification time.
    b.extend_from_slice(&timescale.to_be_bytes());
    b.extend_from_slice(&duration.to_be_bytes());
    b.extend_from_slice(&[0x55, 0xC4, 0x00, 0x00]); // Language + pre-defined.
    b
}

fn build_hdlr_vide() -> Vec<u8> {
    let name = b"VideoHandler\0";
    // Content: 4 (ver/flags) + 4 (pre-defined) + 4 (handler_type) + 12 (reserved) + name = 24 + name
    let mut b = box_start(b"hdlr", 24 + name.len());
    b.extend_from_slice(&[0u8; 4]); // Version, flags.
    b.extend_from_slice(&[0u8; 4]); // Pre-defined.
    b.extend_from_slice(b"vide");
    b.extend_from_slice(&[0u8; 12]); // Reserved.
    b.extend_from_slice(name);
    b
}

fn build_minf(w: u16, h: u16, sc: u32) -> Vec<u8> {
    let vmhd = build_vmhd();
    let dinf = build_dinf();
    let stbl = build_stbl(w, h, sc);
    let mut b = box_start(b"minf", vmhd.len() + dinf.len() + stbl.len());
    b.extend_from_slice(&vmhd);
    b.extend_from_slice(&dinf);
    b.extend_from_slice(&stbl);
    b
}

fn build_vmhd() -> Vec<u8> {
    let mut b = box_start(b"vmhd", 12);
    b.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]); // Version 0, flags=1.
    b.extend_from_slice(&[0u8; 8]); // Graphics mode + opcolor.
    b
}

fn build_dinf() -> Vec<u8> {
    let mut url_b = box_start(b"url ", 4);
    url_b.extend_from_slice(&[0x00, 0x00, 0x00, 0x01]); // Self-contained.

    let mut dref = box_start(b"dref", 8 + url_b.len());
    dref.extend_from_slice(&[0u8; 4]); // Version, flags.
    dref.extend_from_slice(&1u32.to_be_bytes()); // Entry count.
    dref.extend_from_slice(&url_b);

    let mut dinf = box_start(b"dinf", dref.len());
    dinf.extend_from_slice(&dref);
    dinf
}

fn build_stbl(w: u16, h: u16, sc: u32) -> Vec<u8> {
    let stsd = build_stsd_avc1(w, h);
    let stts = build_stts(sc);
    let stsc = build_stsc(sc);
    let stsz = build_stsz(sc);
    let stco = build_stco();
    let total = stsd.len() + stts.len() + stsc.len() + stsz.len() + stco.len();
    let mut b = box_start(b"stbl", total);
    b.extend_from_slice(&stsd);
    b.extend_from_slice(&stts);
    b.extend_from_slice(&stsc);
    b.extend_from_slice(&stsz);
    b.extend_from_slice(&stco);
    b
}

fn build_stsd_avc1(width: u16, height: u16) -> Vec<u8> {
    let avcc = build_avcc();
    let mut avc1 = box_start(b"avc1", 78 + avcc.len());
    avc1.extend_from_slice(&[0u8; 6]); // Reserved.
    avc1.extend_from_slice(&1u16.to_be_bytes()); // Data reference index.
    avc1.extend_from_slice(&[0u8; 16]); // Pre-defined + reserved.
    avc1.extend_from_slice(&width.to_be_bytes());
    avc1.extend_from_slice(&height.to_be_bytes());
    avc1.extend_from_slice(&0x0048_0000u32.to_be_bytes()); // H resolution.
    avc1.extend_from_slice(&0x0048_0000u32.to_be_bytes()); // V resolution.
    avc1.extend_from_slice(&[0u8; 4]); // Reserved.
    avc1.extend_from_slice(&1u16.to_be_bytes()); // Frame count.
    avc1.extend_from_slice(&[0u8; 32]); // Compressor name.
    avc1.extend_from_slice(&0x0018u16.to_be_bytes()); // Depth.
    avc1.extend_from_slice(&0xFFFFu16.to_be_bytes()); // Pre-defined.
    avc1.extend_from_slice(&avcc);

    let mut stsd = box_start(b"stsd", 8 + avc1.len());
    stsd.extend_from_slice(&[0u8; 4]); // Version, flags.
    stsd.extend_from_slice(&1u32.to_be_bytes()); // Entry count.
    stsd.extend_from_slice(&avc1);
    stsd
}

fn build_avcc() -> Vec<u8> {
    let sps: &[u8] = &[0x67, 0x42, 0xC0, 0x1E, 0xD9, 0x00, 0xA0, 0x47, 0xFE, 0xC8];
    let pps: &[u8] = &[0x68, 0xCE, 0x38, 0x80];
    let mut d = Vec::new();
    d.push(0x01); // Config version.
    d.extend_from_slice(&sps[1..4]); // Profile, compat, level.
    d.push(0xFF); // NALU length size.
    d.push(0xE1); // Num SPS.
    d.extend_from_slice(&(sps.len() as u16).to_be_bytes());
    d.extend_from_slice(sps);
    d.push(0x01); // Num PPS.
    d.extend_from_slice(&(pps.len() as u16).to_be_bytes());
    d.extend_from_slice(pps);
    let mut b = box_start(b"avcC", d.len());
    b.extend_from_slice(&d);
    b
}

fn build_stts(sc: u32) -> Vec<u8> {
    let mut b = box_start(b"stts", 16);
    b.extend_from_slice(&[0u8; 4]);
    b.extend_from_slice(&1u32.to_be_bytes());
    b.extend_from_slice(&sc.to_be_bytes());
    b.extend_from_slice(&33u32.to_be_bytes()); // ~30fps at timescale 1000.
    b
}

fn build_stsc(sc: u32) -> Vec<u8> {
    let mut b = box_start(b"stsc", 20);
    b.extend_from_slice(&[0u8; 4]);
    b.extend_from_slice(&1u32.to_be_bytes());
    b.extend_from_slice(&1u32.to_be_bytes());
    b.extend_from_slice(&sc.to_be_bytes());
    b.extend_from_slice(&1u32.to_be_bytes());
    b
}

fn build_stsz(sc: u32) -> Vec<u8> {
    let mut b = box_start(b"stsz", 12);
    b.extend_from_slice(&[0u8; 4]);
    b.extend_from_slice(&100u32.to_be_bytes()); // Uniform sample size.
    b.extend_from_slice(&sc.to_be_bytes());
    b
}

fn build_stco() -> Vec<u8> {
    let mut b = box_start(b"stco", 12);
    b.extend_from_slice(&[0u8; 4]);
    b.extend_from_slice(&1u32.to_be_bytes());
    b.extend_from_slice(&0u32.to_be_bytes());
    b
}

pub fn build_mdat(sample_count: u32) -> Vec<u8> {
    let data_size = sample_count as usize * 100;
    let mut b = box_start(b"mdat", data_size);
    b.resize(b.len() + data_size, 0x00);
    b
}
