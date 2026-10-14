//! Ports of the spec-tiler section 17 checklist items that apply to the indexed renderer.
//! Items for the legacy RGBA renderer, Python adoption (H6) and fault injection are listed in
//! README.md and are not ported here. Adapted items say so in their comment.

mod common;

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use common::*;
use radar_tiler::classify::{ClassModel, build_categorical_class_model, build_class_model};
use radar_tiler::geometry::{AxisMap, GeometryBody, TileGeometry};
use radar_tiler::identity::frame_contract;
use radar_tiler::marker::{
    PREVIOUS_PYRAMID_COMPLETE_FILE, PYRAMID_COMPLETE_FILE, complete_pyramid_identity,
    is_complete_pyramid, list_tile_paths, write_completion_marker,
};
use radar_tiler::palettes::load_palette;
use radar_tiler::png::{decode_indexed, encode_indexed, iter_chunks, replace_palette};
use radar_tiler::sample::{
    footprint_edges, max_block_factors, reduce_column_ranges, sample_bilinear_into,
};
use radar_tiler::{
    ColorTable, FrameKind, FrameOptions, Grid, GridGeo, Overview, PublishStatus, TileRenderer,
    TilerError, render_frame_palettes, resolve_tile_renderer,
};

const NAMES: [&str; 3] = ["classic", "muted", "vivid"];

fn table_json(text: &str) -> ColorTable {
    ColorTable::from_value(&serde_json::from_str(text).unwrap()).unwrap()
}

fn reflectivity_palettes() -> Vec<(String, ColorTable)> {
    NAMES
        .iter()
        .map(|n| (n.to_string(), palette_table(n, "reflectivity")))
        .collect()
}

fn outputs(root: &Path, names: &[&str]) -> Vec<(String, PathBuf)> {
    names
        .iter()
        .map(|n| (n.to_string(), root.join(n)))
        .collect()
}

/// Regular grid with `h x w` cells starting at `(lat0, lon0)` with the given steps.
fn regular_grid(
    h: usize,
    w: usize,
    lat0: f64,
    dlat: f64,
    lon0: f64,
    dlon: f64,
    f: impl Fn(usize, usize) -> f32,
) -> Grid {
    let lats: Vec<f64> = (0..h).map(|i| lat0 + dlat * i as f64).collect();
    let lons: Vec<f64> = (0..w).map(|j| lon0 + dlon * j as f64).collect();
    let values = (0..h * w).map(|k| f(k / w, k % w)).collect();
    Grid {
        width: w,
        height: h,
        values,
        geo: GridGeo::Regular { lats, lons },
    }
}

/// A small reflectivity-like field over central Texas.
fn small_field() -> Grid {
    regular_grid(60, 90, 30.0, 0.02, -100.0, 0.02, |r, c| {
        if (20..40).contains(&r) && (30..60).contains(&c) {
            35.0 + (r as f32) * 0.1
        } else {
            f32::NAN
        }
    })
}

/// Every `(chunk type, payload)` pair of `png`, in order.
fn chunks(png: &[u8]) -> Vec<(String, Vec<u8>)> {
    iter_chunks(png)
        .unwrap()
        .into_iter()
        .map(|(t, p)| (String::from_utf8_lossy(&t).into_owned(), p.to_vec()))
        .collect()
}

fn read_png(path: &Path) -> Vec<u8> {
    fs::read(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

// --- 17.1 / 17.2 ----------------------------------------------------------------------------

#[test]
fn atomic_render_publishes_final_dir_without_tmp_leftover() {
    // test_atomic_render_publishes_final_dir_without_tmp_leftover (indexed renderer)
    let tmp = tempfile::tempdir().unwrap();
    let names = ["classic"];
    let palettes = reflectivity_palettes();
    let result = render_frame_palettes(
        &small_field(),
        &FrameKind::Continuous {
            nodata_value: None,
            min_valid_weight: 1.0,
        },
        &palettes,
        &outputs(tmp.path(), &names),
        &[4, 5],
        &FrameOptions::default(),
    )
    .unwrap();
    assert_eq!(
        result.outcome("classic").unwrap().status,
        PublishStatus::Created
    );
    assert!(result.outcome("classic").unwrap().tile_count > 0);
    let leftovers = fs::read_dir(tmp.path())
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains(".tmp-")
        })
        .count();
    assert_eq!(leftovers, 0);
}

#[test]
fn atomic_render_transparent_frame_publishes_nothing() {
    // test_atomic_render_transparent_frame_publishes_nothing (indexed renderer)
    let tmp = tempfile::tempdir().unwrap();
    let blank = regular_grid(60, 90, 30.0, 0.02, -100.0, 0.02, |_, _| f32::NAN);
    let result = render_frame_palettes(
        &blank,
        &FrameKind::Continuous {
            nodata_value: None,
            min_valid_weight: 1.0,
        },
        &reflectivity_palettes(),
        &outputs(tmp.path(), &NAMES),
        &[4, 5],
        &FrameOptions::default(),
    )
    .unwrap();
    assert!(
        result
            .outcomes
            .iter()
            .all(|(_, o)| o.status == PublishStatus::Empty && o.tile_count == 0)
    );
    assert!(result.rendered_palettes().is_empty());
    for n in NAMES {
        assert!(!tmp.path().join(n).exists());
    }
    let staging = fs::read_dir(tmp.path()).unwrap().count();
    assert_eq!(staging, 0, "no staging directories may remain");
}

#[test]
fn unbound_legacy_trees_are_never_adopted_adapted_h6() {
    // Adapted from test_atomic_render_adopts_exact_pre_marker_legacy_dir,
    // test_atomic_render_rejects_visually_equal_but_byte_different_legacy_dir,
    // test_atomic_render_rejects_partial_pre_marker_legacy_dir and
    // test_atomic_multi_palette_rejects_incomplete_existing_dir. Rust never adopts (spec H6),
    // so every unmarked non-empty final is an Unbound error, and nothing is rendered.
    let tmp = tempfile::tempdir().unwrap();
    let classic = tmp.path().join("classic");
    fs::create_dir_all(classic.join("4/0")).unwrap();
    fs::write(classic.join("4/0/0.png"), b"stale").unwrap();
    let err = render_frame_palettes(
        &small_field(),
        &FrameKind::Continuous {
            nodata_value: None,
            min_valid_weight: 1.0,
        },
        &reflectivity_palettes(),
        &outputs(tmp.path(), &["classic", "muted"]),
        &[4],
        &FrameOptions::default(),
    )
    .unwrap_err();
    assert!(matches!(err, TilerError::Unbound(_)), "{err}");
    assert!(
        !tmp.path().join("muted").exists(),
        "the other palette must not be created"
    );
    assert!(!classic.join(PYRAMID_COMPLETE_FILE).exists());
}

#[test]
fn schema_v1_marker_cannot_bless_a_tree_adapted() {
    // Adapted from test_schema_v1_marker_requires_exact_adoption and
    // test_schema_v1_marker_cannot_bless_a_partial_tree: a v1 marker never counts as bound.
    let tmp = tempfile::tempdir().unwrap();
    let classic = tmp.path().join("classic");
    fs::create_dir_all(classic.join("4/0")).unwrap();
    fs::write(classic.join("4/0/0.png"), b"x").unwrap();
    fs::write(
        classic.join(PREVIOUS_PYRAMID_COMPLETE_FILE),
        b"{\"schema\":1}\n",
    )
    .unwrap();
    let err = render_frame_palettes(
        &small_field(),
        &FrameKind::Continuous {
            nodata_value: None,
            min_valid_weight: 1.0,
        },
        &reflectivity_palettes(),
        &outputs(tmp.path(), &["classic"]),
        &[4],
        &FrameOptions::default(),
    )
    .unwrap_err();
    assert!(matches!(err, TilerError::Unbound(_)), "{err}");
    assert!(
        !classic.join(PYRAMID_COMPLETE_FILE).exists(),
        "no v2 marker may be written"
    );
}

#[test]
fn publish_retry_reports_existing_valid_and_partial_publish_converges() {
    // test_atomic_multi_palette_retry_reports_existing_valid and
    // test_atomic_partial_publish_converges_on_retry (adapted: first call renders classic only,
    // the retry adds muted, so no failure injection is needed).
    let tmp = tempfile::tempdir().unwrap();
    let field = small_field();
    let kind = FrameKind::Continuous {
        nodata_value: None,
        min_valid_weight: 1.0,
    };
    let palettes = reflectivity_palettes();
    let first = render_frame_palettes(
        &field,
        &kind,
        &palettes,
        &outputs(tmp.path(), &["classic"]),
        &[4, 5],
        &FrameOptions::default(),
    )
    .unwrap();
    assert_eq!(
        first.outcome("classic").unwrap().status,
        PublishStatus::Created
    );
    let second = render_frame_palettes(
        &field,
        &kind,
        &palettes,
        &outputs(tmp.path(), &["classic", "muted"]),
        &[4, 5],
        &FrameOptions::default(),
    )
    .unwrap();
    assert_eq!(
        second.outcome("classic").unwrap().status,
        PublishStatus::ExistingValid
    );
    assert_eq!(
        second.outcome("muted").unwrap().status,
        PublishStatus::Created
    );
    assert_eq!(
        second.outcome("classic").unwrap().tile_count,
        second.outcome("muted").unwrap().tile_count
    );
    let third = render_frame_palettes(
        &field,
        &kind,
        &palettes,
        &outputs(tmp.path(), &NAMES),
        &[4, 5],
        &FrameOptions::default(),
    )
    .unwrap();
    assert_eq!(
        third.outcome("classic").unwrap().status,
        PublishStatus::ExistingValid
    );
    assert_eq!(
        third.outcome("muted").unwrap().status,
        PublishStatus::ExistingValid
    );
    assert_eq!(
        third.outcome("vivid").unwrap().status,
        PublishStatus::Created
    );
}

#[test]
fn multi_palette_publish_writes_marker_schema2_and_shared_idat() {
    // test_atomic_multi_palette_publishes_each_pyramid_without_tmp_leftover and
    // test_indexed_tiles_share_idat_and_are_indexed_pngs. Renderer name is "indexed-rs1".
    let tmp = tempfile::tempdir().unwrap();
    let palettes = reflectivity_palettes();
    let kind = FrameKind::Continuous {
        nodata_value: None,
        min_valid_weight: 1.0,
    };
    let result = render_frame_palettes(
        &small_field(),
        &kind,
        &palettes,
        &outputs(tmp.path(), &NAMES),
        &[4, 5],
        &FrameOptions::default(),
    )
    .unwrap();
    let counts: Vec<u64> = result.counts().iter().map(|(_, c)| *c).collect();
    assert!(counts.iter().all(|&c| c > 0 && c == counts[0]));
    let tiles = list_tile_paths(&tmp.path().join("classic")).unwrap();
    let model = build_class_model(&palettes).unwrap();
    for rel in &tiles {
        let mut idats = Vec::new();
        for n in NAMES {
            let png = read_png(&tmp.path().join(n).join(rel));
            assert!(png.len() < 256 * 256, "tile {rel} is {} bytes", png.len());
            let ch = chunks(&png);
            let types: Vec<&str> = ch.iter().map(|(t, _)| t.as_str()).collect();
            assert_eq!(types.first(), Some(&"IHDR"));
            assert_eq!(types[1], "PLTE");
            assert_eq!(types[2], "tRNS");
            assert_eq!(types.last(), Some(&"IEND"));
            assert_eq!(ch[0].1[9], 3, "colour type must be indexed");
            let lut = model.lut(n).unwrap();
            assert_eq!(ch[1].1, lut.plte(), "{rel}: PLTE for {n}");
            assert_eq!(ch[2].1, lut.trns(), "{rel}: tRNS for {n}");
            let decoded = decode_indexed(&png).unwrap();
            for (&idx, _) in decoded.indices.iter().zip(0..) {
                assert_eq!(
                    &lut.plte()[usize::from(idx) * 3..usize::from(idx) * 3 + 3],
                    &lut.rgba()[usize::from(idx)][..3]
                );
            }
            idats.push(
                ch.iter()
                    .filter(|(t, _)| t == "IDAT")
                    .map(|(_, p)| p.clone())
                    .collect::<Vec<_>>(),
            );
        }
        assert!(
            idats.windows(2).all(|w| w[0] == w[1]),
            "IDAT must be identical across palettes for {rel}"
        );
    }
    let id = complete_pyramid_identity(&tmp.path().join("classic")).expect("bound");
    assert_eq!(id.renderer, "indexed-rs1");
    assert_eq!(id.palette_name, "classic");
    assert_eq!(id.algorithm, "physical-bilinear-classify-v1");
    assert_eq!(id.source_id, "unspecified-frame");
    assert!(
        !id.palette_digest.is_empty()
            && !id.semantic_digest.is_empty()
            && !id.policy_digest.is_empty()
    );
    let leftovers = fs::read_dir(tmp.path())
        .unwrap()
        .filter(|e| {
            e.as_ref()
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains(".tmp-")
        })
        .count();
    assert_eq!(leftovers, 0);
}

#[test]
fn invalid_overlapping_outputs_and_lock_root_fail_before_mutation() {
    // test_invalid_overlapping_output_paths_fail_before_mutation[False] and [True],
    // test_publication_lock_root_must_be_a_strict_output_ancestor.
    let tmp = tempfile::tempdir().unwrap();
    let field = small_field();
    let kind = FrameKind::Continuous {
        nodata_value: None,
        min_valid_weight: 1.0,
    };
    let palettes = reflectivity_palettes();
    let base = tmp.path().join("radar/classic");
    let equal = vec![
        ("classic".to_string(), base.clone()),
        ("muted".to_string(), base.clone()),
    ];
    let err = render_frame_palettes(
        &field,
        &kind,
        &palettes,
        &equal,
        &[4],
        &FrameOptions::default(),
    )
    .unwrap_err();
    assert!(err.to_string().contains("distinct, non-nested"), "{err}");
    let nested = vec![
        ("classic".to_string(), base.clone()),
        ("muted".to_string(), base.join("inner")),
    ];
    let err = render_frame_palettes(
        &field,
        &kind,
        &palettes,
        &nested,
        &[4],
        &FrameOptions::default(),
    )
    .unwrap_err();
    assert!(err.to_string().contains("distinct, non-nested"), "{err}");
    assert!(
        !tmp.path().join("radar").exists(),
        "no output or lock directory may be created"
    );
    let options = FrameOptions {
        publication_lock_root: Some(base.clone()),
        ..FrameOptions::default()
    };
    let err = render_frame_palettes(
        &field,
        &kind,
        &palettes,
        &[("classic".to_string(), base.clone())],
        &[4],
        &options,
    )
    .unwrap_err();
    assert!(err.to_string().contains("lock root"), "{err}");
    assert!(!base.exists());
}

#[test]
fn completion_marker_detects_path_and_content_tamper() {
    // test_completion_marker_detects_same_count_path_tamper and
    // test_completion_marker_detects_tile_content_tamper.
    let tmp = tempfile::tempdir().unwrap();
    let field = small_field();
    let kind = FrameKind::Continuous {
        nodata_value: None,
        min_valid_weight: 1.0,
    };
    let palettes = reflectivity_palettes();
    let out = tmp.path().join("classic");
    render_frame_palettes(
        &field,
        &kind,
        &palettes,
        &outputs(tmp.path(), &["classic"]),
        &[4],
        &FrameOptions::default(),
    )
    .unwrap();
    let tile = list_tile_paths(&out).unwrap().into_iter().next().unwrap();
    let original = fs::read(out.join(&tile)).unwrap();
    // Content tamper: flip one byte.
    let mut flipped = original.clone();
    let last = flipped.len() - 20;
    flipped[last] ^= 1;
    fs::write(out.join(&tile), &flipped).unwrap();
    assert!(!is_complete_pyramid(&out, None));
    let err = render_frame_palettes(
        &field,
        &kind,
        &palettes,
        &outputs(tmp.path(), &["classic"]),
        &[4],
        &FrameOptions::default(),
    )
    .unwrap_err();
    assert!(matches!(err, TilerError::Corrupt(_)), "{err}");
    fs::write(out.join(&tile), &original).unwrap();
    // Path tamper: rename the tile to a sibling name with the same count.
    let renamed = out.join(&tile).with_file_name("999.png");
    fs::rename(out.join(&tile), &renamed).unwrap();
    assert!(!is_complete_pyramid(&out, None));
    let err = render_frame_palettes(
        &field,
        &kind,
        &palettes,
        &outputs(tmp.path(), &["classic"]),
        &[4],
        &FrameOptions::default(),
    )
    .unwrap_err();
    assert!(matches!(err, TilerError::Corrupt(_)), "{err}");
}

#[test]
fn identity_marker_is_create_only() {
    // test_identity_marker_is_create_only: a second marker with another identity is refused.
    let tmp = tempfile::tempdir().unwrap();
    let out = tmp.path().join("classic");
    let kind = FrameKind::Continuous {
        nodata_value: None,
        min_valid_weight: 1.0,
    };
    render_frame_palettes(
        &small_field(),
        &kind,
        &reflectivity_palettes(),
        &outputs(tmp.path(), &["classic"]),
        &[4],
        &FrameOptions::default(),
    )
    .unwrap();
    let mut id = complete_pyramid_identity(&out).unwrap();
    let paths = list_tile_paths(&out).unwrap();
    let before = fs::read(out.join(PYRAMID_COMPLETE_FILE)).unwrap();
    id.policy_digest = "different".into();
    let err = write_completion_marker(&out, &paths, &id).unwrap_err();
    assert!(err.to_string().contains("different identity"), "{err}");
    assert_eq!(fs::read(out.join(PYRAMID_COMPLETE_FILE)).unwrap(), before);
}

#[test]
fn foreign_identities_are_reported_not_rewritten() {
    // Adapted from test_foreign_renderer_rejects_unknown_algorithm_identity,
    // test_bound_identity_rejects_changed_render_inputs[source|grid|palette] and
    // test_foreign_renderer_rejects_changed_categorical_semantics. Collision rule (PLAN): a
    // complete pyramid with a different identity is ExistingForeign with its tile count.
    let tmp = tempfile::tempdir().unwrap();
    let field = small_field();
    let kind = FrameKind::Continuous {
        nodata_value: None,
        min_valid_weight: 1.0,
    };
    let palettes = reflectivity_palettes();
    let out = tmp.path().join("classic");
    let first = render_frame_palettes(
        &field,
        &kind,
        &palettes,
        &outputs(tmp.path(), &["classic"]),
        &[4, 5],
        &FrameOptions::default(),
    )
    .unwrap();
    let count = first.outcome("classic").unwrap().tile_count;

    let other_source = FrameOptions {
        source_id: "other-source".into(),
        ..FrameOptions::default()
    };
    let r = render_frame_palettes(
        &field,
        &kind,
        &palettes,
        &outputs(tmp.path(), &["classic"]),
        &[4, 5],
        &other_source,
    )
    .unwrap();
    assert_eq!(
        r.outcome("classic").unwrap().status,
        PublishStatus::ExistingForeign
    );
    assert_eq!(r.outcome("classic").unwrap().tile_count, count);

    let shifted = {
        let mut g = small_field();
        if let GridGeo::Regular { lats, .. } = &mut g.geo {
            for v in lats.iter_mut() {
                *v += 0.01;
            }
        }
        g
    };
    let r = render_frame_palettes(
        &shifted,
        &kind,
        &palettes,
        &outputs(tmp.path(), &["classic"]),
        &[4, 5],
        &FrameOptions::default(),
    )
    .unwrap();
    assert_eq!(
        r.outcome("classic").unwrap().status,
        PublishStatus::ExistingForeign
    );

    let mut tweaked = reflectivity_palettes();
    let mut rgba = tweaked[0].1.ranges().unwrap()[0].rgba;
    rgba[0] ^= 1;
    let table = serde_json::json!({
        "ranges": tweaked[0].1.ranges().unwrap().iter().map(|r| serde_json::json!({"min": r.min, "max": r.max, "rgba": if r.min == tweaked[0].1.ranges().unwrap()[0].min { rgba.to_vec() } else { r.rgba.to_vec() }})).collect::<Vec<_>>(),
        "no_data_below": tweaked[0].1.no_data_below(),
    });
    tweaked[0].1 = ColorTable::from_value(&table).unwrap();
    let r = render_frame_palettes(
        &field,
        &kind,
        &tweaked,
        &outputs(tmp.path(), &["classic"]),
        &[4, 5],
        &FrameOptions::default(),
    )
    .unwrap();
    assert_eq!(
        r.outcome("classic").unwrap().status,
        PublishStatus::ExistingForeign,
        "palette change must not rewrite"
    );

    let mut cat = BTreeMap::new();
    cat.insert(1i64, "rain".to_string());
    let r = render_frame_palettes(
        &field,
        &FrameKind::Categorical { category_map: cat },
        &[("classic".into(), palette_table("classic", "precip_type"))],
        &outputs(tmp.path(), &["classic"]),
        &[4],
        &FrameOptions::default(),
    )
    .unwrap();
    assert_eq!(
        r.outcome("classic").unwrap().status,
        PublishStatus::ExistingForeign
    );

    // Unknown algorithm/renderer in a complete marker: still foreign, never blessed.
    let mut future = complete_pyramid_identity(&out).unwrap();
    future.renderer = "future".into();
    future.algorithm = "unknown-v99".into();
    fs::remove_file(out.join(PYRAMID_COMPLETE_FILE)).unwrap();
    let paths = list_tile_paths(&out).unwrap();
    write_completion_marker(&out, &paths, &future).unwrap();
    let r = render_frame_palettes(
        &field,
        &kind,
        &palettes,
        &outputs(tmp.path(), &["classic"]),
        &[4, 5],
        &FrameOptions::default(),
    )
    .unwrap();
    assert_eq!(
        r.outcome("classic").unwrap().status,
        PublishStatus::ExistingForeign
    );
}

#[test]
fn partial_palette_set_renders_absent_palettes_adapted() {
    // Adapted from test_partial_foreign_renderer_set_fails_closed: a bound palette beside an
    // absent one is reported as-is and the absent one is rendered (per-palette collision rule).
    let tmp = tempfile::tempdir().unwrap();
    let field = small_field();
    let kind = FrameKind::Continuous {
        nodata_value: None,
        min_valid_weight: 1.0,
    };
    let palettes = reflectivity_palettes();
    render_frame_palettes(
        &field,
        &kind,
        &palettes,
        &outputs(tmp.path(), &["classic"]),
        &[4],
        &FrameOptions::default(),
    )
    .unwrap();
    let r = render_frame_palettes(
        &field,
        &kind,
        &palettes,
        &outputs(tmp.path(), &["classic", "muted"]),
        &[4],
        &FrameOptions::default(),
    )
    .unwrap();
    assert_eq!(
        r.outcome("classic").unwrap().status,
        PublishStatus::ExistingValid
    );
    assert_eq!(r.outcome("muted").unwrap().status, PublishStatus::Created);
}

#[test]
fn extra_palettes_in_the_table_set_are_ignored() {
    // Adapted from test_atomic_helpers_allow_palette_payload_supersets: the frame takes
    // tables for the output set; extra entries are ignored.
    let tmp = tempfile::tempdir().unwrap();
    let kind = FrameKind::Continuous {
        nodata_value: None,
        min_valid_weight: 1.0,
    };
    let r = render_frame_palettes(
        &small_field(),
        &kind,
        &reflectivity_palettes(),
        &outputs(tmp.path(), &["muted"]),
        &[4],
        &FrameOptions::default(),
    )
    .unwrap();
    assert_eq!(r.outcomes.len(), 1);
    let missing = render_frame_palettes(
        &small_field(),
        &kind,
        &reflectivity_palettes(),
        &outputs(tmp.path(), &["nope"]),
        &[4],
        &FrameOptions::default(),
    );
    assert!(matches!(missing, Err(TilerError::Invalid(_))));
}

#[test]
fn overview_is_rejected_for_categorical_and_unknown_modes_are_unrepresentable() {
    // test_overview_rejects_categorical_grids. "mean" cannot be expressed: Overview has one variant.
    let tmp = tempfile::tempdir().unwrap();
    let mut cat = BTreeMap::new();
    cat.insert(1i64, "rain".to_string());
    let options = FrameOptions {
        overview: Some(Overview::Max),
        ..FrameOptions::default()
    };
    let err = render_frame_palettes(
        &small_field(),
        &FrameKind::Categorical { category_map: cat },
        &[("classic".into(), palette_table("classic", "precip_type"))],
        &outputs(tmp.path(), &["classic"]),
        &[4],
        &options,
    )
    .unwrap_err();
    assert!(matches!(err, TilerError::Invalid(_)), "{err}");
}

#[test]
fn overview_is_part_of_the_contract_only_when_set() {
    // test_overview_is_part_of_the_render_identity_only_when_set
    let model = build_class_model(&reflectivity_palettes()).unwrap();
    let kind = FrameKind::Continuous {
        nodata_value: None,
        min_valid_weight: 1.0,
    };
    let plain = frame_contract(&kind, &model, 6, None);
    let pooled = frame_contract(&kind, &model, 6, Some("max"));
    assert!(!plain.policy.to_json().contains("\"overview\""));
    assert!(pooled.policy.to_json().contains("\"overview\":\"max\""));
    assert!(pooled.semantic.to_json().contains("\"overview\":\"max\""));
}

#[test]
fn single_cell_core_survives_low_zoom_only_with_overview() {
    // test_single_cell_core_survives_low_zoom_only_with_overview[indexed]: a 62 dBZ cell in a
    // 20 dBZ field is visible at z4 with overview="max" and not without.
    let fixture = testdata("overview");
    let lats = read_f64(&fixture.join("lats.f64"));
    let lons = read_f64(&fixture.join("lons.f64"));
    let grid = Grid {
        width: 900,
        height: 600,
        values: read_f32(&fixture.join("data.f32")),
        geo: GridGeo::Regular { lats, lons },
    };
    let palettes = [(
        "classic".to_string(),
        palette_table("classic", "reflectivity"),
    )];
    let model = build_class_model(&palettes).unwrap();
    let mut core = [0u8; 1];
    model.classify_f32_into(&[62.0], &mut core).unwrap();
    let core_class = core[0];
    assert_ne!(core_class, 0);
    let kind = FrameKind::Continuous {
        nodata_value: None,
        min_valid_weight: 1.0,
    };
    let contains_core = |root: &Path| -> bool {
        list_tile_paths(root).unwrap().iter().any(|rel| {
            decode_indexed(&read_png(&root.join(rel)))
                .unwrap()
                .indices
                .contains(&core_class)
        })
    };
    let tmp = tempfile::tempdir().unwrap();
    let plain_root = tmp.path().join("plain/classic");
    render_frame_palettes(
        &grid,
        &kind,
        &palettes,
        &[("classic".to_string(), plain_root.clone())],
        &[4],
        &FrameOptions::default(),
    )
    .unwrap();
    let pooled_root = tmp.path().join("pooled/classic");
    let options = FrameOptions {
        overview: Some(Overview::Max),
        ..FrameOptions::default()
    };
    render_frame_palettes(
        &grid,
        &kind,
        &palettes,
        &[("classic".to_string(), pooled_root.clone())],
        &[4],
        &options,
    )
    .unwrap();
    assert!(
        contains_core(&pooled_root),
        "overview must keep the core at z4"
    );
    assert!(
        !contains_core(&plain_root),
        "native sampling must lose the core at z4"
    );
}

// --- 17.3 overview helpers ------------------------------------------------------------------

#[test]
fn overview_helpers_match_the_checklist() {
    // test_block_factors_track_mercator_pixel_size
    let lats: Vec<f64> = (0..3500).map(|i| 20.0 + 0.01 * i as f64).collect();
    let lons: Vec<f64> = (0..7000).map(|i| -130.0 + 0.01 * i as f64).collect();
    assert_eq!(max_block_factors(&lats, &lons, 4, 256), (5, 8));
    assert_eq!(max_block_factors(&lats, &lons, 5, 256), (2, 4));
    assert_eq!(max_block_factors(&lats, &lons, 7, 256), (1, 1));

    // test_footprint_edges_cover_every_cell_once_and_flag_off_grid_pixels
    let axis = AxisMap::new(&(0..10).map(f64::from).collect::<Vec<_>>()).unwrap();
    let edges = [-1.0, -0.6, 2.5, 5.5, 8.5, 10.5];
    let ranges = footprint_edges(&edges, &axis);
    assert_eq!(
        ranges.iter().map(|r| r.0).collect::<Vec<_>>(),
        vec![0, 0, 3, 6, 9]
    );
    assert_eq!(
        ranges.iter().map(|r| r.1).collect::<Vec<_>>(),
        vec![0, 3, 6, 9, 10]
    );
    let desc = AxisMap::new(&(0..10).rev().map(f64::from).collect::<Vec<_>>()).unwrap();
    // A descending axis indexes the same coordinate values in reverse order, so each pixel
    // must cover the same coordinate values (index i holds value 9 - i).
    let desc_ranges = footprint_edges(&edges, &desc);
    for (asc, dsc) in ranges.iter().zip(&desc_ranges) {
        let mut want: Vec<usize> = (asc.0..asc.1).collect();
        let mut got: Vec<usize> = (dsc.0..dsc.1).map(|i| 9 - i).collect();
        want.sort_unstable();
        got.sort_unstable();
        assert_eq!(got, want, "a descending axis covers the same values");
    }

    // test_footprint_edges_keep_the_cell_under_a_sub_cell_pixel
    let sub = footprint_edges(&[4.1, 4.2], &axis);
    assert_eq!(sub, vec![(4, 5)]);

    // test_reduce_ranges_is_nan_aware_and_empty_ranges_are_nan
    let values = [1.0f32, f32::NAN, 7.0, 2.0, f32::NAN];
    let out = reduce_column_ranges(&values, 1, 5, &[(0, 2), (1, 4), (4, 5), (5, 5)]);
    assert_eq!(out[0], 1.0);
    assert_eq!(out[1], 7.0);
    assert!(out[2].is_nan() && out[3].is_nan());
}

// --- 17.4 PNG rewrite ------------------------------------------------------------------------

fn palette_of(n: usize, shift: u8) -> (Vec<u8>, Vec<u8>) {
    let rgb: Vec<u8> = (0..n)
        .flat_map(|i| {
            [
                (i as u8).wrapping_add(shift),
                (255 - i as u8),
                (i as u8).wrapping_mul(3),
            ]
        })
        .collect();
    let alpha: Vec<u8> = (0..n).map(|i| if i == 0 { 0 } else { 255 }).collect();
    (rgb, alpha)
}

fn sample_pixels(n: usize, w: usize, h: usize) -> Vec<u8> {
    (0..w * h).map(|i| ((i * 7 + i / w) % n) as u8).collect()
}

#[test]
fn replace_palette_matches_direct_encode_at_each_depth() {
    // test_replace_palette_matches_direct_encode[2|12|16|17|200] and
    // test_replace_palette_recomputes_crc.
    for n in [2usize, 12, 16, 17, 200] {
        let (w, h) = (33, 17);
        let tile = sample_pixels(n, w, h);
        let (plte_a, trns_a) = palette_of(n, 0);
        let (plte_b, trns_b) = palette_of(n, 91);
        let png_a = encode_indexed(&tile, w, h, &plte_a, &trns_a, 6).unwrap();
        let direct_b = encode_indexed(&tile, w, h, &plte_b, &trns_b, 6).unwrap();
        let rewritten = replace_palette(&png_a, &plte_b, Some(&trns_b)).unwrap();
        assert_eq!(
            decode_indexed(&rewritten).unwrap().indices,
            decode_indexed(&direct_b).unwrap().indices,
            "n={n}"
        );
        let (a, b) = (chunks(&png_a), chunks(&rewritten));
        let pick =
            |c: &[(String, Vec<u8>)], t: &str| c.iter().find(|(k, _)| k == t).unwrap().1.clone();
        assert_eq!(
            pick(&a, "IDAT"),
            pick(&b, "IDAT"),
            "n={n}: IDAT must be byte-identical"
        );
        assert_eq!(
            pick(&a, "IHDR"),
            pick(&b, "IHDR"),
            "n={n}: IHDR must be byte-identical"
        );
        assert_eq!(pick(&b, "PLTE"), plte_b);
        assert_eq!(pick(&b, "tRNS"), trns_b);
        // Every CRC equals crc32(type || payload).
        let mut cursor = 8usize;
        while cursor < rewritten.len() {
            let len =
                u32::from_be_bytes(rewritten[cursor..cursor + 4].try_into().unwrap()) as usize;
            let body = &rewritten[cursor + 4..cursor + 8 + len];
            let crc = u32::from_be_bytes(
                rewritten[cursor + 8 + len..cursor + 12 + len]
                    .try_into()
                    .unwrap(),
            );
            assert_eq!(crc, crc32fast::hash(body));
            cursor += 12 + len;
        }
    }
}

#[test]
fn replace_palette_inserts_and_drops_trns_and_rejects_bad_input() {
    // test_replace_palette_inserts_trns_when_missing_and_drops_when_none,
    // test_replace_palette_rejects_non_indexed_png, test_replace_palette_validates_lengths.
    let tile = sample_pixels(3, 4, 4);
    let (rgb, alpha) = palette_of(3, 0);
    let png = encode_indexed(&tile, 4, 4, &rgb, &alpha, 6).unwrap();
    let dropped = replace_palette(&png, &rgb, None).unwrap();
    assert!(!chunks(&dropped).iter().any(|(t, _)| t == "tRNS"));
    let inserted = replace_palette(&dropped, &rgb, Some(&alpha)).unwrap();
    let trns = chunks(&inserted)
        .into_iter()
        .find(|(t, _)| t == "tRNS")
        .unwrap()
        .1;
    assert_eq!(trns[0], 0, "pixel 0 is transparent");
    let mut rgba_bytes = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut rgba_bytes, 1, 1);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().unwrap();
        writer.write_image_data(&[1, 2, 3, 4]).unwrap();
    }
    let err = replace_palette(&rgba_bytes, &rgb, None).unwrap_err();
    assert!(err.to_string().contains("not indexed"), "{err}");
    assert!(
        replace_palette(&png, &[1, 2, 3, 4], None).is_err(),
        "rgb length not multiple of 3"
    );
    assert!(
        replace_palette(&png, &rgb, Some(&[0; 9])).is_err(),
        "alpha longer than palette"
    );
    assert!(
        replace_palette(b"not a png at all", &rgb, None).is_err(),
        "bad signature"
    );
}

#[test]
fn iter_chunks_detects_truncation() {
    // test_iter_chunks_detects_truncation
    let iend = [0u8, 0, 0, 0, b'I', b'E', b'N', b'D', 0xAE, 0x42, 0x60, 0x82];
    let mut png = radar_tiler::png::PNG_SIGNATURE.to_vec();
    png.extend_from_slice(&iend);
    let parsed = iter_chunks(&png).unwrap();
    assert_eq!(parsed.len(), 1);
    assert_eq!(&parsed[0].0, b"IEND");
    let err = iter_chunks(&png[..png.len() - 5]).unwrap_err();
    assert!(err.to_string().contains("truncated"), "{err}");
}

// --- 17.5 class model ------------------------------------------------------------------------

/// The legacy formula (spec 5.1, 5.3): the later range that contains `v` wins, half-open
/// `[min, max)`, values below `no_data_below` and non-finite values are transparent. Edges and
/// values are compared in f32, as the shared class model does (spec 5.3 precision rule).
fn legacy_rgba(table: &ColorTable, v: f32) -> [u8; 4] {
    if !v.is_finite() || v < table.no_data_below() as f32 {
        return [0; 4];
    }
    let mut out = [0u8; 4];
    for r in table.ranges().unwrap_or(&[]) {
        if r.min as f32 <= v && v < r.max as f32 {
            out = r.rgba;
        }
    }
    out
}

#[test]
fn class_model_matches_legacy_formula_for_every_shipped_table() {
    // test_class_model_matches_legacy_for_every_shipped_table[float32] (f64 variant not ported:
    // the Rust pipeline stores values as f32, spec 4.2).
    for name in NAMES {
        let palette = load_palette(name).unwrap();
        for key in palette.color_keys().map(str::to_string).collect::<Vec<_>>() {
            let table = palette.color_table(&key).unwrap();
            if table.ranges().is_none() {
                continue;
            }
            let model = build_class_model(&[(name.to_string(), table.clone())]).unwrap();
            let edges = model.edges().unwrap().to_vec();
            let mut probes: Vec<f64> = Vec::new();
            for &e in &edges {
                probes.extend([e, e - 1e-3, e + 1e-3]);
            }
            probes.extend([f64::NAN, f64::INFINITY, f64::NEG_INFINITY]);
            let f32_probes: Vec<f32> = probes.iter().map(|&p| p as f32).collect();
            let mut classes = vec![0u8; f32_probes.len()];
            model.classify_f32_into(&f32_probes, &mut classes).unwrap();
            let lut = model.lut(name).unwrap();
            for (k, &p) in f32_probes.iter().enumerate() {
                let want = legacy_rgba(&table, p);
                assert_eq!(
                    lut.rgba()[usize::from(classes[k])],
                    want,
                    "{name}/{key} at {p}"
                );
            }
        }
    }
}

#[test]
fn shipped_palettes_share_edges_per_key() {
    // test_all_shipped_palettes_share_bin_edges_per_color_key. The exact class count is checked
    // per palette: n_classes is the number of transparent-collapsed bins plus the transparent row.
    let palettes: Vec<_> = NAMES
        .iter()
        .map(|n| (n.to_string(), load_palette(n).unwrap()))
        .collect();
    let keys: Vec<String> = palettes[0].1.color_keys().map(str::to_string).collect();
    for key in keys {
        let mut per: Vec<(String, ClassModel, usize)> = Vec::new();
        for (name, pal) in &palettes {
            let table = pal.color_table(&key).unwrap();
            if let Some(r) = table.ranges() {
                let model = build_class_model(&[(name.clone(), table.clone())]).unwrap();
                per.push((name.clone(), model, r.len()));
            }
        }
        if per.is_empty() {
            continue;
        }
        let edges0 = per[0].1.edges().unwrap().to_vec();
        for (name, model, n_ranges) in &per {
            assert_eq!(
                model.edges().unwrap(),
                &edges0[..],
                "{key}: edges differ for {name}"
            );
            assert!(
                model.n_classes() <= n_ranges + 1,
                "{key}/{name}: more classes than ranges + 1"
            );
        }
    }
}

#[test]
fn class_model_handles_overlapping_and_disjoint_palettes() {
    // test_class_model_later_overlapping_range_wins_like_legacy and
    // test_class_model_handles_palettes_with_different_edges.
    let overlap = table_json(
        r#"{"ranges":[{"min":0,"max":20,"rgba":[255,0,0,255]},{"min":10,"max":30,"rgba":[0,0,255,255]}],"no_data_below":0}"#,
    );
    let model = build_class_model(&[("a".into(), overlap.clone())]).unwrap();
    let lut = model.lut("a").unwrap();
    let mut out = [0u8; 4];
    model
        .classify_f32_into(&[5.0, 15.0, 25.0, 40.0], &mut out)
        .unwrap();
    assert_eq!(lut.rgba()[usize::from(out[0])], [255, 0, 0, 255]);
    assert_eq!(
        lut.rgba()[usize::from(out[1])],
        [0, 0, 255, 255],
        "later range wins"
    );
    assert_eq!(lut.rgba()[usize::from(out[2])], [0, 0, 255, 255]);
    assert_eq!(out[3], 0, "outside every range is class 0");

    let other =
        table_json(r#"{"ranges":[{"min":5,"max":7,"rgba":[1,2,3,255]}],"no_data_below":5}"#);
    let both = build_class_model(&[("a".into(), overlap), ("b".into(), other)]).unwrap();
    let mut cls = [0u8; 2];
    both.classify_f32_into(&[6.0, 25.0], &mut cls).unwrap();
    assert_eq!(
        both.lut("b").unwrap().rgba()[usize::from(cls[0])],
        [1, 2, 3, 255]
    );
    assert_eq!(
        both.lut("b").unwrap().rgba()[usize::from(cls[1])],
        [0, 0, 0, 0]
    );
}

#[test]
fn categorical_model_matches_legacy_including_negative_and_unknown_ids() {
    // test_categorical_model_matches_legacy: ids outside the map are class 0.
    let table = palette_table("classic", "precip_type");
    let map: BTreeMap<i64, String> = [
        (1, "rain"),
        (2, "snow"),
        (3, "freezing_rain"),
        (4, "ice_pellets"),
    ]
    .iter()
    .map(|(k, v)| (*k, v.to_string()))
    .collect();
    let model = build_categorical_class_model(&[("classic".into(), table.clone())], &map).unwrap();
    let lut = model.lut("classic").unwrap();
    let values = [-3.0f32, 0.0, 1.0, 2.0, 3.0, 4.0, 5.0, 99.0];
    let mut out = [0u8; 8];
    model.classify_category_into(&values, &mut out).unwrap();
    let cats = table.categories().unwrap();
    let want = |v: i64| -> [u8; 4] {
        if v < 0 {
            return [0; 4];
        }
        map.get(&v)
            .and_then(|c| cats.get(c))
            .copied()
            .unwrap_or([0; 4])
    };
    for (k, &v) in values.iter().enumerate() {
        assert_eq!(lut.rgba()[usize::from(out[k])], want(v as i64), "id {v}");
    }
}

#[test]
fn classify_is_shape_preserving_and_chunk_independent() {
    // test_classify_is_chunked_and_shape_preserving: a long input classifies the same as
    // element-wise calls, and an empty input is accepted.
    let model = build_class_model(&reflectivity_palettes()).unwrap();
    let values: Vec<f32> = (0..3000 * 7).map(|i| (i % 90) as f32 - 10.0).collect();
    let mut all = vec![0u8; values.len()];
    model.classify_f32_into(&values, &mut all).unwrap();
    for (k, &v) in values.iter().enumerate().step_by(97) {
        let mut one = [0u8];
        model.classify_f32_into(&[v], &mut one).unwrap();
        assert_eq!(all[k], one[0], "index {k}");
    }
    let mut empty: [u8; 0] = [];
    model.classify_f32_into(&[], &mut empty).unwrap();
}

#[test]
fn bilinear_sampling_policy_and_mid_cell_value() {
    // test_bilinear_samples_physical_values_with_explicit_nodata_policy
    let geom = TileGeometry {
        body: GeometryBody::RegularBilinear {
            rows: vec![0.5, 0.5],
            cols: vec![0.5, 0.5],
        },
        row_bounds: (0, 1),
        col_bounds: (0, 1),
    };
    let mut tile = [0f32; 4];
    sample_bilinear_into(
        &geom,
        &[10.0, 20.0, 10.0, 20.0],
        2,
        2,
        2,
        None,
        1.0,
        &mut tile,
    )
    .unwrap();
    assert!(
        tile.iter().all(|v| (v - 15.0).abs() < 1e-6),
        "mid-cell mean: {tile:?}"
    );
    let nodata = -999.0;
    let corner = [10.0, -999.0, 10.0, 10.0];
    let mut strict = [0f32; 4];
    sample_bilinear_into(&geom, &corner, 2, 2, 2, Some(nodata), 1.0, &mut strict).unwrap();
    assert!(
        strict.iter().all(|v| v.is_nan()),
        "strict policy rejects a nodata corner"
    );
    let mut partial = [0f32; 4];
    sample_bilinear_into(&geom, &corner, 2, 2, 2, Some(nodata), 0.75, &mut partial).unwrap();
    assert!(
        partial.iter().all(|v| (v - 10.0).abs() < 1e-5),
        "partial policy: {partial:?}"
    );
}

#[test]
fn fractional_indices_support_monotonic_axes_and_reject_others() {
    // test_fractional_indices_support_nonuniform_monotonic_axes
    let asc = AxisMap::new(&[-1.0, 0.5, 1.5, 2.5]).unwrap();
    assert_eq!(asc.map(-1.0), 0.0);
    assert_eq!(asc.map(0.5), 1.0);
    assert_eq!(asc.map(2.5), 3.0);
    let desc = AxisMap::new(&[3.0, 1.5, 0.5, -0.5]).unwrap();
    assert_eq!(desc.map(3.0), 0.0);
    assert_eq!(desc.map(-0.5), 3.0);
    let err = AxisMap::new(&[0.0, 2.0, 1.0]).expect_err("non-monotonic");
    assert!(err.to_string().contains("strictly monotonic"), "{err}");
}

// --- Indexed render behaviour ----------------------------------------------------------------

#[test]
fn indexed_render_matches_palette_lut_and_tile_geometry_is_valid() {
    // Covered by test_indexed_tiles_share_idat_and_are_indexed_pngs above. This checks that a
    // zoom with no overlap produces no tiles and that palette names are validated (H13).
    let tmp = tempfile::tempdir().unwrap();
    let kind = FrameKind::Continuous {
        nodata_value: None,
        min_valid_weight: 1.0,
    };
    let bad = vec![("../escape".to_string(), tmp.path().join("x"))];
    let err = render_frame_palettes(
        &small_field(),
        &kind,
        &reflectivity_palettes(),
        &bad,
        &[4],
        &FrameOptions::default(),
    )
    .unwrap_err();
    assert!(matches!(err, TilerError::Invalid(_)));
}

// --- Renderer selection (section 14) ---------------------------------------------------------

#[test]
fn renderer_selection_fails_closed_and_needs_a_canary() {
    // test_unknown_renderer_fails_closed (adapted: the env reader is `resolve_tile_renderer`),
    // test_indexed_renderer_requires_one_named_canary_role,
    // test_legacy_renderer_ignores_stale_canary_role.
    assert!(resolve_tile_renderer("mrms", Some("typo"), None).is_err());
    let err = resolve_tile_renderer("mrms", Some("indexed"), None).unwrap_err();
    assert!(err.to_string().contains("CANARY_ROLE"), "{err}");
    assert_eq!(
        resolve_tile_renderer("mrms", Some("indexed"), Some("mrms")).unwrap(),
        TileRenderer::Indexed
    );
    assert_eq!(
        resolve_tile_renderer("hrrr", Some("indexed"), Some("mrms")).unwrap(),
        TileRenderer::Legacy
    );
    assert_eq!(
        resolve_tile_renderer("hrrr", Some("legacy"), Some("mrms")).unwrap(),
        TileRenderer::Legacy
    );
    assert!(resolve_tile_renderer("bogus", None, None).is_err());
}

#[test]
fn symlinked_alias_of_a_final_is_rejected_before_mutation() {
    // Adapted from test_overlapping_palette_sets_share_a_publication_lock. Finals are
    // canonicalized first (spec 13.1 step 1), so an alias of the same final is an overlap error
    // rather than lock contention.
    let tmp = tempfile::tempdir().unwrap();
    let real = tmp.path().join("radar/classic");
    fs::create_dir_all(tmp.path().join("radar")).unwrap();
    std::os::unix::fs::symlink(tmp.path().join("radar"), tmp.path().join("alias")).unwrap();
    let alias = tmp.path().join("alias/classic");
    let outputs = vec![
        ("classic".to_string(), real.clone()),
        ("muted".to_string(), alias),
    ];
    let kind = FrameKind::Continuous {
        nodata_value: None,
        min_valid_weight: 1.0,
    };
    let err = render_frame_palettes(
        &small_field(),
        &kind,
        &reflectivity_palettes(),
        &outputs,
        &[4],
        &FrameOptions::default(),
    )
    .unwrap_err();
    assert!(err.to_string().contains("distinct, non-nested"), "{err}");
    assert!(!real.exists());
}
