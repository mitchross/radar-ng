use radar_level2::{decode::Decoder, *};
use std::io::Write;

fn radial(number: u16, status: u8, elevation: u8, gates: u16, bits: u8) -> Vec<u8> {
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
fn header() -> Vec<u8> {
    let mut h = b"AR2V0006.001".to_vec();
    h.extend_from_slice(&20765u32.to_be_bytes());
    h.extend_from_slice(&0u32.to_be_bytes());
    h.extend_from_slice(b"KGRR");
    h
}
fn record(bytes: &[u8]) -> Vec<u8> {
    let mut encoder = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::fast());
    encoder.write_all(bytes).unwrap();
    let bz = encoder.finish().unwrap();
    let mut out = (bz.len() as i32).to_be_bytes().to_vec();
    out.extend(bz);
    out
}
fn volume(radials: &[u8]) -> Vec<u8> {
    let mut v = header();
    v.extend(record(radials));
    v
}
fn sweep(elevation: u8, gates: u16, bits: u8) -> Vec<u8> {
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
#[test]
fn real_decoder_enforces_radial_gate_sweep_and_continuity_limits() {
    for bits in [8, 16] {
        let v = volume(&sweep(1, GATE_LIMIT as u16, bits));
        let mut d = Decoder::new("KGRR");
        let s = d.push(&v).unwrap();
        d.finish().unwrap();
        assert_eq!(s.len(), 1);
        assert_eq!(s[0].rays[0].moments[0].as_ref().unwrap().first_gate_m, -125);
        assert!(
            Decoder::new("KGRR")
                .push(&volume(&sweep(1, GATE_LIMIT as u16 + 1, bits)))
                .is_err()
        );
    }
    let rays: Vec<_> = (1..=RADIAL_LIMIT as u16 + 1)
        .flat_map(|n| radial(n, if n == 1 { 0 } else { 1 }, 1, 2, 8))
        .collect();
    assert!(
        Decoder::new("KGRR")
            .push(&volume(&rays))
            .unwrap_err()
            .to_string()
            .contains("radial count")
    );
    let mut d = Decoder::new("KGRR");
    d.push(&header()).unwrap();
    for el in 1..=SWEEP_LIMIT as u8 {
        assert_eq!(d.push(&record(&sweep(el, 2, 8))).unwrap().len(), 1);
    }
    assert!(
        d.push(&record(&sweep(33, 2, 8)))
            .unwrap_err()
            .to_string()
            .contains("sweep count")
    );
    let missing: Vec<_> = (1..=360)
        .filter(|n| *n != 100)
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
                1,
                2,
                8,
            )
        })
        .collect();
    assert!(Decoder::new("KGRR").push(&volume(&missing)).is_err());
    let partial = volume(&radial(1, 0, 1, 2, 8));
    let mut d = Decoder::new("KGRR");
    assert!(d.push(&partial).unwrap().is_empty());
    assert!(d.finish().is_err());
}
#[test]
fn corrupt_truncated_and_expansion_bombs_fail_closed() {
    let v = volume(&sweep(1, 2, 8));
    for n in [0, 4, 23, 24, 25, v.len() - 1] {
        assert!(Decoder::new("KGRR").push(&v[..n]).is_err() || n == 24);
    }
    let mut corrupt = v.clone();
    corrupt[30] ^= 0xff;
    assert!(Decoder::new("KGRR").push(&corrupt).is_err());
    let mut bomb = header();
    bomb.extend(record(&vec![0; RECORD_LIMIT + 1]));
    assert!(
        Decoder::new("KGRR")
            .push(&bomb)
            .unwrap_err()
            .to_string()
            .contains("expanded byte limit")
    );
    let mut oversized = header();
    oversized.extend_from_slice(&i32::MIN.to_be_bytes());
    assert!(Decoder::new("KGRR").push(&oversized).is_err());
}

#[test]
fn decoded_arrays_stop_before_exceeding_pool_reservation() {
    let mut d = Decoder::new("KGRR");
    d.push(&header()).unwrap();
    let mut retained = Vec::new();
    for ordinal in 1..=32 {
        match d.push(&record(&sweep(ordinal, GATE_LIMIT as u16, 16))) {
            Ok(sweeps) => retained.extend(sweeps),
            Err(error) => {
                assert!(error.to_string().contains("decoded byte limit"), "{error}");
                assert!(!retained.is_empty());
                return;
            }
        }
    }
    panic!("decoded arrays exceeded their reservation");
}

#[test]
fn higher_ref_cannot_replace_a_low_cut_without_ref() {
    let mut low = sweep(1, 2, 8);
    let frame = radial(1, 0, 1, 2, 8).len();
    for ray in low.chunks_exact_mut(frame) {
        ray[28 + 40..28 + 44].copy_from_slice(b"DVEL");
    }
    let mut d = Decoder::new("KGRR");
    assert!(d.push(&volume(&low)).unwrap().is_empty());
    let mut higher = sweep(2, 2, 8);
    for ray in higher.chunks_exact_mut(frame) {
        ray[28 + 24..28 + 28].copy_from_slice(&1.5f32.to_be_bytes());
    }
    assert!(d.push(&record(&higher)).unwrap().is_empty());
    d.finish().unwrap();
}
