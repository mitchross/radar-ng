use radar_level2::{decode::Decoder, *};

mod common;
use common::*;

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

#[test]
fn azimuth_coverage_rejects_duplicates_reversal_and_mixed_spacing() {
    let frame = radial(1, 0, 1, 2, 8).len();
    for mode in 0..5 {
        let mut data = sweep(1, 2, 8);
        for (i, ray) in data.chunks_exact_mut(frame).enumerate() {
            let az = match mode {
                0 => 0.0,
                1 if i == 100 => 99.0,
                2 => (360.0 - i as f32) % 360.0,
                3 => (i as f32 * 2.0) % 360.0,
                _ => i as f32,
            };
            ray[40..44].copy_from_slice(&az.to_be_bytes());
            if mode == 4 && i == 100 {
                ray[48] = 1;
            }
        }
        assert!(
            Decoder::new("KGRR").push(&volume(&data)).is_err(),
            "mode {mode}"
        );
    }
    let mut data = sweep(1, 2, 8);
    for (i, ray) in data.chunks_exact_mut(frame).enumerate() {
        let az = i as f32 * 0.99 + if i % 2 == 0 { 0.08 } else { 0.0 };
        ray[40..44].copy_from_slice(&az.to_be_bytes());
    }
    let mut decoder = Decoder::new("KGRR");
    let decoded = decoder.push(&volume(&data)).unwrap();
    decoder.finish().unwrap();
    assert_eq!(decoded[0].missing_sectors_deg.len(), 1);
    assert_eq!(decoded[0].missing_sectors_deg[0][1], 0.08);
}
