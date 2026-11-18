//! Per-hour HRRR CPU work (spec §12.2): extract the
//! variables, render every layer that has a palette, and write the grid dumps. Runs on the blocking pool.

use std::collections::BTreeMap;
use std::path::PathBuf;

use radar_geo::{Crs, Grid, GridGeo};
use radar_grib::decode::{DecodedMessage, find_message};
use radar_grib::hrrr::HrrrVar;
use radar_grib::mrms::ensure_south_up;
use radar_grib::units;
use radar_tiler::identity::{
    FrameKind, RENDERER, frame_contract, frame_identities, frame_identity_is_compatible,
    frame_model,
};
use radar_tiler::palettes::Palette;
use radar_tiler::{
    ColorTable, FrameOptions, TypePlane, complete_pyramid_identity, render_frame_palettes,
};

use super::dump::{Resampler, safe_dump, target_axes};
use super::state::{RADAR_HRRR, layer_color_key};
use super::{HRRR_BEAT, HRRR_ZOOMS, PRECIP_TYPE_CATEGORIES, PRECIP_TYPE_GRID};
use crate::config::IngestConfig;

/// Category id to name map for the precip-type layer.
fn category_map() -> BTreeMap<i64, String> {
    PRECIP_TYPE_CATEGORIES
        .iter()
        .map(|(id, name)| (*id, name.to_string()))
        .collect()
}

/// The palettes that carry `key`, with their color tables, in the loader's order.
pub fn tables_for_key(palettes: &[Palette], key: &str) -> Vec<(String, ColorTable)> {
    palettes
        .iter()
        .filter(|p| p.color_keys().any(|k| k == key))
        .filter_map(|p| p.color_table(key).ok().map(|t| (p.name().to_string(), t)))
        .collect()
}

/// Palette tables for HRRR: each `PALETTES` name, with missing ones warned and skipped; `classic`
/// when nothing loads. Duplicates are dropped (first occurrence wins).
pub fn load_hrrr_palettes(names: &[String]) -> Vec<Palette> {
    let mut seen = std::collections::BTreeSet::new();
    let mut out = Vec::new();
    for name in names {
        if !seen.insert(name.clone()) {
            continue;
        }
        match radar_tiler::load_palette(name) {
            Ok(p) => out.push(p),
            Err(err) => tracing::warn!(palette = %name, err = %err, "palette_missing"),
        }
    }
    if out.is_empty()
        && let Ok(p) = radar_tiler::load_palette("classic")
    {
        out.push(p);
    }
    out
}

/// Render one layer for every palette that has its color key (spec §12.2).
/// Returns true only when every such palette rendered (D2 replicated: a partial or empty set is not
/// published for this hour).
#[allow(
    clippy::too_many_arguments,
    reason = "mirrors the Python _render_per_palette signature; a parameter struct would only rename the same fields"
)]
pub fn render_layer(
    cfg: &IngestConfig,
    layer: &str,
    run_id: &str,
    valid: &str,
    grid: &Grid,
    palettes: &[Palette],
    color_key: &str,
    categorical: bool,
) -> anyhow::Result<bool> {
    render_layer_typed(
        cfg,
        layer,
        run_id,
        valid,
        grid,
        palettes,
        color_key,
        categorical,
        None,
    )
}

/// [`render_layer`] with a precipitation-type plane on `grid`'s cells: the PNGs use the rain,
/// snow, mix and ice ramps, and the packed slot stores typed codes.
#[allow(
    clippy::too_many_arguments,
    reason = "render_layer plus the type plane"
)]
pub fn render_layer_typed(
    cfg: &IngestConfig,
    layer: &str,
    run_id: &str,
    valid: &str,
    grid: &Grid,
    palettes: &[Palette],
    color_key: &str,
    categorical: bool,
    types: Option<TypePlane>,
) -> anyhow::Result<bool> {
    let tables = tables_for_key(palettes, color_key);
    if tables.is_empty() {
        return Ok(false);
    }
    let kind = if categorical {
        FrameKind::Categorical {
            category_map: category_map(),
        }
    } else {
        FrameKind::Continuous {
            nodata_value: None,
            min_valid_weight: 1.0,
        }
    };
    let rel = format!("runs/{run_id}/{valid}");
    let tile_layer = cfg.paths.tile_layer_dir(layer);
    let dirs: Vec<(String, PathBuf)> = tables
        .iter()
        .map(|(n, _)| (n.clone(), tile_layer.join(n).join(&rel)))
        .collect();
    let options = FrameOptions {
        source_id: format!("hrrr:{layer}:{rel}"),
        publication_lock_root: Some(tile_layer.clone()),
        overview: None,
        types: types.clone(),
        // An all-day stargazing hour has no tiles; publish it empty so the hour resumes.
        keep_empty: super::state::KEEP_EMPTY_LAYERS.contains(&layer),
        ..FrameOptions::default()
    };
    let keep_empty = options.keep_empty;
    let result = render_frame_palettes(grid, &kind, &tables, &dirs, &HRRR_ZOOMS, &options)?;
    let rendered = if keep_empty {
        result.outcomes.len() == tables.len()
            && result
                .outcomes
                .iter()
                .all(|(_, o)| o.status != radar_tiler::PublishStatus::Empty)
    } else {
        result.rendered_palettes().len() == tables.len()
    };
    let typed = super::packed::is_typed(layer, cfg.precip_type);
    if rendered
        && cfg.packed_tiles
        && let Err(err) =
            super::packed::pack_hour_typed(&cfg.paths, layer, run_id, valid, grid, typed, types)
    {
        tracing::warn!(layer, run_id, valid, err = %format!("{err:#}"), "packed_failed");
    }
    Ok(rendered)
}

/// Whether a stored identity is acceptable for the current contract (spec H13 to H18).
///
/// A complete pyramid whose identity matches this renderer's contract passes. A pyramid written by a
/// foreign renderer passes when its algorithm, palette, tile spec, and semantics match (policy is
/// excluded, spec §6.8.2).
fn identity_ok(
    stored: &radar_tiler::PyramidIdentity,
    source_id: &str,
    tables: &[(String, ColorTable)],
    name: &str,
    kind: &FrameKind,
) -> bool {
    let png_level = FrameOptions::default().png_compress_level;
    let Ok(compatible) = frame_identity_is_compatible(
        stored,
        source_id,
        tables,
        name,
        &HRRR_ZOOMS,
        256,
        kind,
        png_level,
        None,
    ) else {
        return false;
    };
    if compatible {
        return true;
    }
    if stored.renderer == RENDERER {
        return false;
    }
    let Ok(model) = frame_model(kind, tables) else {
        return false;
    };
    let contract = frame_contract(kind, &model, png_level, None);
    let Some((_, table)) = tables.iter().find(|(n, _)| n == name) else {
        return false;
    };
    let Ok(expected) = frame_identities(
        source_id,
        &stored.source_digest,
        &stored.grid_spec_digest,
        &HRRR_ZOOMS,
        256,
        &contract,
        &[(name.to_string(), table.clone())],
    ) else {
        return false;
    };
    let expected = &expected[0].1;
    stored.algorithm == expected.algorithm
        && stored.palette_digest == expected.palette_digest
        && stored.tile_spec_digest == expected.tile_spec_digest
        && stored.semantic_digest == expected.semantic_digest
}

/// Layers already complete for this hour (spec H13 to H18). A layer counts
/// only when every palette that carries its key has a complete, acceptable pyramid. Ignores
/// `HRRR_ENABLED_LAYERS` (D17).
pub fn existing_rendered_layers(
    cfg: &IngestConfig,
    palettes: &[Palette],
    run_id: &str,
    valid: &str,
) -> Vec<String> {
    let rel = format!("runs/{run_id}/{valid}");
    let mut out = Vec::new();
    for layer in crate::config::HRRR_LAYERS {
        let Some(key) = layer_color_key(layer) else {
            continue;
        };
        let tables = tables_for_key(palettes, key);
        if tables.is_empty() {
            continue;
        }
        let kind = if layer == "precip-type" {
            FrameKind::Categorical {
                category_map: category_map(),
            }
        } else {
            FrameKind::Continuous {
                nodata_value: None,
                min_valid_weight: 1.0,
            }
        };
        let source_id = format!("hrrr:{layer}:{rel}");
        let all_ok = tables.iter().all(|(name, _)| {
            let path = cfg.paths.tile_layer_dir(layer).join(name).join(&rel);
            match complete_pyramid_identity(&path) {
                Some(stored) => identity_ok(&stored, &source_id, &tables, name, &kind),
                None => false,
            }
        });
        if all_ok {
            out.push(layer.to_string());
        }
    }
    out
}

/// A decoded message, normalised to south-up (the same rule as MRMS, applied to HRRR too).
fn extract(bytes: &[u8], var: HrrrVar) -> Option<DecodedMessage> {
    match find_message(bytes, &var.selector()) {
        Ok(Some(mut msg)) => {
            ensure_south_up(&mut msg.grid);
            Some(msg)
        }
        Ok(None) => None,
        Err(err) => {
            tracing::warn!(err = %err, matched = var.key(), "extract_failed");
            None
        }
    }
}

/// [`extract`] for accumulations: the latest window (APCP's last hour, not the run total).
fn extract_latest_window(bytes: &[u8], var: HrrrVar) -> Option<DecodedMessage> {
    match radar_grib::decode::find_latest_window(bytes, &var.selector()) {
        Ok(Some(mut msg)) => {
            ensure_south_up(&mut msg.grid);
            Some(msg)
        }
        Ok(None) => None,
        Err(err) => {
            tracing::warn!(err = %err, matched = var.key(), "extract_failed");
            None
        }
    }
}

/// Owned grid from a message's values, with its geometry.
fn grid_of(values: Vec<f32>, template: &Grid) -> Grid {
    Grid {
        width: template.width,
        height: template.height,
        values,
        geo: template.geo.clone(),
    }
}

/// Process one forecast hour's GRIB file (spec §12.2). Returns the layers
/// rendered, in pipeline order.
pub fn process_hour_sync(
    cfg: &IngestConfig,
    bytes: &[u8],
    run_id: &str,
    valid: &str,
    palettes: &[Palette],
) -> anyhow::Result<Vec<String>> {
    let refc = extract(bytes, HrrrVar::Refc).ok_or_else(|| {
        anyhow::anyhow!("required HRRR variable refc is missing from downloaded GRIB")
    })?;
    let GridGeo::Projected { crs, x, y, .. } = &refc.grid.geo else {
        anyhow::bail!("HRRR message has no projected geometry");
    };
    let crs: Crs = crs.clone();
    let (src_h, src_w) = (refc.grid.height, refc.grid.width);
    let (x, y) = (x.clone(), y.clone());
    let (lats_t, lons_t) = target_axes(&refc.grid, cfg.hrrr_grid_dump_step_deg)?;
    let resampler = Resampler::new(&crs, &x, &y, src_h, src_w, &lats_t, &lons_t)?;
    let enabled = |name: &str| cfg.hrrr_enabled_layers.iter().any(|l| l == name);
    let dump = |layer: &str, data: &[f32], unit: &str| {
        let resampled = resampler.resample(data);
        safe_dump(cfg, layer, valid, &resampled, &lats_t, &lons_t, unit);
    };

    // Fields used by more than one layer are decoded once, and only when a layer needs them.
    let t2m_msg = (enabled("temperature") || enabled("feels-like") || enabled("nice-day"))
        .then(|| extract(bytes, HrrrVar::T2m))
        .flatten();
    let rh2m_msg = (enabled("humidity") || enabled("feels-like") || enabled("nice-day"))
        .then(|| extract(bytes, HrrrVar::Rh2m))
        .flatten();
    let u10_msg = (enabled("feels-like") || enabled("wind") || enabled("nice-day"))
        .then(|| extract(bytes, HrrrVar::U10))
        .flatten();
    let v10_msg = (enabled("feels-like") || enabled("wind") || enabled("nice-day"))
        .then(|| extract(bytes, HrrrVar::V10))
        .flatten();

    let mut rendered = Vec::new();
    // HRRR precip type: always decoded for the `precip-type` grid that colours MRMS ice, and for the
    // classes of `radar-hrrr`; the standalone PNG layer only when enabled.
    let flags: Vec<Option<DecodedMessage>> = [
        HrrrVar::Crain,
        HrrrVar::Csnow,
        HrrrVar::Cfrzr,
        HrrrVar::Cicep,
    ]
    .into_iter()
    .map(|v| extract(bytes, v))
    .collect();
    let flag_slices: Vec<Option<&[f32]>> = flags
        .iter()
        .map(|f| f.as_ref().map(|m| m.grid.values.as_slice()))
        .collect();
    let has_flags = flag_slices.iter().any(Option::is_some);
    let len = src_h * src_w;
    let radar_types = (cfg.precip_type && has_flags)
        .then(|| {
            crate::precip_type::plane_unless_all_rain(crate::precip_type::model_classes(
                flag_slices[0],
                flag_slices[1],
                flag_slices[2],
                flag_slices[3],
                len,
            ))
        })
        .flatten();
    // radar-hrrr (always on).
    if render_layer_typed(
        cfg,
        RADAR_HRRR,
        run_id,
        valid,
        &refc.grid,
        palettes,
        "reflectivity",
        false,
        radar_types,
    )? {
        rendered.push(RADAR_HRRR.to_string());
        dump("radar-hrrr", &refc.grid.values, "dBZ");
    }

    if enabled("temperature")
        && let Some(t2m) = &t2m_msg
    {
        let vals: Vec<f32> = t2m
            .grid
            .values
            .iter()
            .map(|&k| units::kelvin_to_f(k))
            .collect();
        let g = grid_of(vals, &t2m.grid);
        if render_layer(
            cfg,
            "temperature",
            run_id,
            valid,
            &g,
            palettes,
            "temperature",
            false,
        )? {
            rendered.push("temperature".into());
            dump("temperature", &g.values, "degF");
        }
    }
    if enabled("dewpoint")
        && let Some(d) = extract(bytes, HrrrVar::Dpt2m)
    {
        let vals: Vec<f32> = d
            .grid
            .values
            .iter()
            .map(|&k| units::kelvin_to_f(k))
            .collect();
        let g = grid_of(vals, &d.grid);
        if render_layer(
            cfg, "dewpoint", run_id, valid, &g, palettes, "dewpoint", false,
        )? {
            rendered.push("dewpoint".into());
            dump("dewpoint", &g.values, "degF");
        }
    }
    if enabled("humidity")
        && let Some(rh) = &rh2m_msg
        && render_layer(
            cfg, "humidity", run_id, valid, &rh.grid, palettes, "humidity", false,
        )?
    {
        rendered.push("humidity".into());
        dump("humidity", &rh.grid.values, "%");
    }
    if enabled("feels-like")
        && let (Some(t2m), Some(rh), Some(u), Some(v)) = (&t2m_msg, &rh2m_msg, &u10_msg, &v10_msg)
    {
        let vals: Vec<f32> = t2m
            .grid
            .values
            .iter()
            .zip(&rh.grid.values)
            .zip(u.grid.values.iter().zip(&v.grid.values))
            .map(|((&k, &h), (&uc, &vc))| {
                let wind = units::wind_speed_mph(units::ms_to_mph(uc), units::ms_to_mph(vc));
                units::apparent_temperature_f(units::kelvin_to_f(k), h, wind)
            })
            .collect();
        let g = grid_of(vals, &t2m.grid);
        if render_layer(
            cfg,
            "feels-like",
            run_id,
            valid,
            &g,
            palettes,
            "temperature",
            false,
        )? {
            rendered.push("feels-like".into());
            dump("feels-like", &g.values, "degF");
        }
    }
    if enabled("cape")
        && let Some(cape) = extract(bytes, HrrrVar::Cape)
        && render_layer(
            cfg, "cape", run_id, valid, &cape.grid, palettes, "cape", false,
        )?
    {
        rendered.push("cape".into());
        dump("cape", &cape.grid.values, "J/kg");
    }
    if enabled("wind")
        && let (Some(u), Some(v)) = (&u10_msg, &v10_msg)
    {
        let u_mph: Vec<f32> = u.grid.values.iter().map(|&c| units::ms_to_mph(c)).collect();
        let v_mph: Vec<f32> = v.grid.values.iter().map(|&c| units::ms_to_mph(c)).collect();
        let speed: Vec<f32> = u_mph
            .iter()
            .zip(&v_mph)
            .map(|(&a, &b)| units::wind_speed_mph(a, b))
            .collect();
        let g = grid_of(speed, &u.grid);
        if render_layer(
            cfg,
            "wind",
            run_id,
            valid,
            &g,
            palettes,
            "wind_speed",
            false,
        )? {
            rendered.push("wind".into());
            dump("wind", &g.values, "mph");
            let (wu, wv) = resampler.wind_components(&u_mph, &v_mph, u.uv_grid_relative());
            safe_dump(cfg, "wind_u", valid, &wu, &lats_t, &lons_t, "mph");
            safe_dump(cfg, "wind_v", valid, &wv, &lats_t, &lons_t, "mph");
        }
    }
    if enabled("precip-accum")
        && let Some(apcp) = extract(bytes, HrrrVar::Apcp)
    {
        let vals: Vec<f32> = apcp
            .grid
            .values
            .iter()
            .map(|&d| units::mm_to_inches(d))
            .collect();
        let g = grid_of(vals, &apcp.grid);
        if render_layer(
            cfg,
            "precip-accum",
            run_id,
            valid,
            &g,
            palettes,
            "precip_accum",
            false,
        )? {
            rendered.push("precip-accum".into());
            dump("precip-accum", &g.values, "in");
        }
    }
    let tcdc_msg = (enabled("cloud") || enabled("stargazing"))
        .then(|| extract(bytes, HrrrVar::Tcdc))
        .flatten();
    if enabled("cloud")
        && let Some(tcdc) = &tcdc_msg
        && render_layer(
            cfg,
            "cloud",
            run_id,
            valid,
            &tcdc.grid,
            palettes,
            "cloud_cover",
            false,
        )?
    {
        rendered.push("cloud".into());
        dump("cloud", &tcdc.grid.values, "%");
    }
    if enabled("nice-day")
        && let (Some(t2m), Some(rh), Some(u), Some(v)) = (&t2m_msg, &rh2m_msg, &u10_msg, &v10_msg)
    {
        // A missing rain field scores dry rather than dropping the layer for the hour.
        let rain = extract_latest_window(bytes, HrrrVar::Apcp);
        let rain_in = |i: usize| {
            rain.as_ref()
                .and_then(|m| m.grid.values.get(i))
                .map_or(0.0, |&mm| units::mm_to_inches(mm))
        };
        let vals: Vec<f32> = (0..t2m.grid.values.len())
            .map(|i| {
                let t = units::kelvin_to_f(t2m.grid.values[i]);
                let h = rh.grid.values[i];
                let wind = units::wind_speed_mph(
                    units::ms_to_mph(u.grid.values[i]),
                    units::ms_to_mph(v.grid.values[i]),
                );
                let feels = units::apparent_temperature_f(t, h, wind);
                radar_grib::nice_day::nice_day_cell(t, feels, wind, rain_in(i), h)
            })
            .collect();
        let g = grid_of(vals, &t2m.grid);
        if render_layer(
            cfg, "nice-day", run_id, valid, &g, palettes, "nice_day", false,
        )? {
            rendered.push("nice-day".into());
            dump("nice-day", &g.values, "score");
        }
    }
    if enabled("stargazing")
        && let Some(tcdc) = &tcdc_msg
        && let Ok(at) = chrono::DateTime::parse_from_rfc3339(valid)
    {
        let vals = radar_grib::stargazing::stargazing_grid(&tcdc.grid, at.timestamp() as f64);
        let g = grid_of(vals, &tcdc.grid);
        if render_layer(
            cfg,
            "stargazing",
            run_id,
            valid,
            &g,
            palettes,
            "stargazing",
            false,
        )? {
            rendered.push("stargazing".into());
            // A daytime hour has no values to read back.
            if g.values.iter().any(|v| v.is_finite()) {
                dump("stargazing", &g.values, "score");
            }
        }
    }
    if has_flags {
        let cat = units::precip_type_category(
            flag_slices[0],
            flag_slices[1],
            flag_slices[2],
            flag_slices[3],
            len,
        );
        let vals: Vec<f32> = cat.iter().map(|&c| c as f32).collect();
        let nearest = resampler.resample_nearest(&vals);
        safe_dump(
            cfg,
            PRECIP_TYPE_GRID,
            valid,
            &nearest,
            &lats_t,
            &lons_t,
            "category",
        );
        let g = grid_of(vals, &refc.grid);
        if enabled("precip-type")
            && render_layer(
                cfg,
                "precip-type",
                run_id,
                valid,
                &g,
                palettes,
                "precip_type",
                true,
            )?
        {
            rendered.push("precip-type".into());
        }
    }
    Ok(rendered)
}

/// Heartbeat cadence for the hour render (re-exported for the activity).
pub const RENDER_BEAT: std::time::Duration = HRRR_BEAT;
