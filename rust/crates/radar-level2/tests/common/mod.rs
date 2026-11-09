use std::io::Write;

pub fn radial(number: u16, status: u8, elevation: u8, gates: u16, bits: u8) -> Vec<u8> {
    let count = usize::from(gates) * usize::from(bits / 8);
    let len = 40 + 32 + count;
    let mut b = vec![0; 28 + len];
    b[12..14].copy_from_slice(&(((16 + len) / 2) as u16).to_be_bytes());
    b[14] = 8;
    b[15] = 31;
    b[24..26].copy_from_slice(&1u16.to_be_bytes());
    b[26..28].copy_from_slice(&1u16.to_be_bytes());
    let h = &mut b[28..];
    h[..4].copy_from_slice(b"KGRR");
    h[4..8].copy_from_slice(&(u32::from(number) * 10).to_be_bytes());
    h[8..10].copy_from_slice(&20765u16.to_be_bytes());
    h[10..12].copy_from_slice(&number.to_be_bytes());
    h[12..16].copy_from_slice(&((number as f32 - 1.0) % 360.0).to_be_bytes());
    h[18..20].copy_from_slice(&(len as u16).to_be_bytes());
    h[20] = 2;
    h[21] = status;
    h[22] = elevation;
    h[24..28].copy_from_slice(&0.5f32.to_be_bytes());
    h[30..32].copy_from_slice(&1u16.to_be_bytes());
    h[32..36].copy_from_slice(&40u32.to_be_bytes());
    let m = &mut h[40..];
    m[..4].copy_from_slice(b"DREF");
    m[8..10].copy_from_slice(&gates.to_be_bytes());
    m[10..12].copy_from_slice(&(-125i16).to_be_bytes());
    m[12..14].copy_from_slice(&250u16.to_be_bytes());
    m[19] = bits;
    m[20..24].copy_from_slice(&2f32.to_be_bytes());
    m[24..28].copy_from_slice(&66f32.to_be_bytes());
    m[28..].fill(100);
    b
}
pub fn header() -> Vec<u8> {
    let mut h = b"AR2V0006.001".to_vec();
    h.extend_from_slice(&20765u32.to_be_bytes());
    h.extend_from_slice(&0u32.to_be_bytes());
    h.extend_from_slice(b"KGRR");
    h
}
pub fn record(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::fast());
    encoder.write_all(bytes).unwrap();
    let bz = encoder.finish().unwrap();
    let mut out = (bz.len() as i32).to_be_bytes().to_vec();
    out.extend(bz);
    out
}
pub fn volume(radials: &[u8]) -> Vec<u8> {
    let mut v = header();
    v.extend(record(radials));
    v
}
pub fn sweep(elevation: u8, gates: u16, bits: u8) -> Vec<u8> {
    (1..=360)
        .flat_map(|n| {
            radial(
                n,
                if n == 1 {
                    0
                } else if n == 360 {
                    2
                } else {
                    1
                },
                elevation,
                gates,
                bits,
            )
        })
        .collect()
}
