//! GRIB2 message decoding into [`radar_geo::Grid`] (spec-ingest-mrms-hrrr §5, spec-ingest-aux §5.6).
//!
//! Supported grids: template 3.0 (regular lat/lon) and 3.30 (Lambert conformal, spherical Earth).
//! Supported values: whatever the `grib` crate decodes (5.0, 5.2, 5.3, 5.40, 5.41, 5.42, 5.200).
//!
//! Grid orientation is returned as stored in the file. For regular grids `lats` is in stored
//! order (MRMS: north first, descending). Call [`crate::mrms::ensure_south_up`] to get the Python
//! "flip when lats[0] > lats[-1]" normalisation.
//!
//! Longitudes are normalised exactly as the Python does: any value `> 180` becomes `value - 360`.
//! Missing values (bitmap or packed missing) are NaN.
//!
//! Known gap: decoded values come from the `grib` crate, which computes the scale in `f32`. pygrib
//! (eccodes) computes it in `f64` and then casts to `f32`, so the two can differ by one ulp for some
//! packed values. See README "Parity with pygrib".

use grib::def::grib2::GridDefinitionTemplate;
use grib::{Grib2Read, Grib2SubmessageDecoder, LatLons, SubMessage};
use radar_geo::{Crs, Grid, GridGeo};

use crate::error::{GribError, Result};
use crate::packing::{Complex, Scale, expand_bitmap, unpack_complex, unpack_png, unpack_simple};

/// GRIB2 Code table 4.5 value for "entire atmosphere" (eccodes `typeOfLevel=atmosphere`).
pub const SURFACE_ENTIRE_ATMOSPHERE: u8 = 10;
/// GRIB2 Code table 4.5 value for "ground or water surface" (eccodes `typeOfLevel=surface`).
pub const SURFACE_GROUND: u8 = 1;
/// GRIB2 Code table 4.5 value for "specified height above ground" (eccodes `heightAboveGround`).
pub const SURFACE_HEIGHT_ABOVE_GROUND: u8 = 103;

/// Metadata of one GRIB2 message, read from sections 0, 3, 4 and 5 without decoding values.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MessageInfo {
    /// Section 0 discipline (0 = meteorological products).
    pub discipline: u8,
    /// Product definition parameter category (Code table 4.1).
    pub category: Option<u8>,
    /// Product definition parameter number (Code table 4.2).
    pub number: Option<u8>,
    /// Type of the first fixed surface (Code table 4.5).
    pub surface_type: Option<u8>,
    /// Value of the first fixed surface, scale factor applied (pygrib `level`).
    pub level_value: Option<f64>,
    /// Grid definition template number (0 or 30 are the ones this crate decodes).
    pub grid_template: u16,
    /// Data representation template number (Code table 5.0).
    pub data_template: u16,
    /// Resolution and component flags (Flag table 3.3); 0 for grid templates without the field.
    pub resolution_flags: u8,
}

/// Selects messages by discipline, category, number, first-surface type and level value.
///
/// Every field that is `Some` must match. This replaces the Python selector dict
/// (`shortName`, `typeOfLevel`, `level`) with the GRIB2 codes those names stand for, so it does not
/// depend on the eccodes version's name tables. See the README for the mapping.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MessageSelector {
    /// Discipline (Section 0).
    pub discipline: Option<u8>,
    /// Parameter category (Table 4.1).
    pub category: Option<u8>,
    /// Parameter number (Table 4.2).
    pub number: Option<u8>,
    /// A second accepted parameter number for the same field. HRRR's categorical precipitation types
    /// were NCEP-local 0.1.192..195 and now have WMO numbers 0.1.33..36; eccodes names both the same.
    pub number_alt: Option<u8>,
    /// First fixed surface type (Table 4.5).
    pub surface_type: Option<u8>,
    /// First fixed surface value (scale factor applied).
    pub level_value: Option<f64>,
}

impl MessageSelector {
    /// Selector for discipline/category/number only.
    pub fn parameter(discipline: u8, category: u8, number: u8) -> Self {
        Self {
            discipline: Some(discipline),
            category: Some(category),
            number: Some(number),
            ..Self::default()
        }
    }

    /// Also accept parameter number `alt`.
    pub fn or_number(mut self, alt: u8) -> Self {
        self.number_alt = Some(alt);
        self
    }

    /// Adds a first-surface constraint.
    pub fn at_level(mut self, surface_type: u8, value: f64) -> Self {
        self.surface_type = Some(surface_type);
        self.level_value = Some(value);
        self
    }

    /// True when every constrained field matches `info`.
    pub fn matches(&self, info: &MessageInfo) -> bool {
        self.discipline.is_none_or(|d| d == info.discipline)
            && self.category.is_none_or(|c| info.category == Some(c))
            && self
                .number
                .is_none_or(|n| info.number == Some(n) || self.number_alt.is_some_and(|a| info.number == Some(a)))
            && self
                .surface_type
                .is_none_or(|s| info.surface_type == Some(s))
            // The ground surface (type 1) has no meaningful value; files leave it missing or 0.
            && self.level_value.is_none_or(|v| {
                info.level_value == Some(v)
                    || (info.surface_type == Some(1) && v == 0.0 && info.level_value.is_none())
            })
    }
}

/// A decoded message: the grid plus the metadata a caller may need.
#[derive(Debug, Clone, PartialEq)]
pub struct DecodedMessage {
    /// Decoded field, orientation as stored.
    pub grid: Grid,
    /// Section metadata of the message.
    pub info: MessageInfo,
}

impl DecodedMessage {
    /// True when u/v winds are resolved relative to the grid axes (Python: `flags & 8`).
    pub fn uv_grid_relative(&self) -> bool {
        self.info.resolution_flags & 8 != 0
    }
}

/// Decodes the first message of `bytes` into a grid (pygrib `grbs[1]`).
///
/// The grid is in stored orientation. Longitudes are normalised to [-180, 180].
pub fn decode_message(bytes: &[u8]) -> Result<Grid> {
    Ok(decode_first_message(bytes)?.grid)
}

/// Decodes the first message and returns its metadata too.
pub fn decode_first_message(bytes: &[u8]) -> Result<DecodedMessage> {
    let file = parse_file(bytes)?;
    let (_, sub) = file
        .iter()
        .next()
        .ok_or_else(|| GribError::Parse("file contains no GRIB2 messages".into()))?;
    let info = info_of(&sub)?;
    decode_submessage(bytes, sub, info)
}

/// Returns the first message that matches `selector`, decoding only that message.
///
/// `Ok(None)` means no message matched. A decode failure of a matching message is an `Err`;
/// the Python code logs and returns None in that case. Callers that want the Python behaviour map
/// the error to None and log it.
pub fn find_message(bytes: &[u8], selector: &MessageSelector) -> Result<Option<DecodedMessage>> {
    let file = parse_file(bytes)?;
    for (_, sub) in file.iter() {
        let info = info_of(&sub)?;
        if selector.matches(&info) {
            return decode_submessage(bytes, sub, info).map(Some);
        }
    }
    Ok(None)
}

/// Lists the metadata of every message in `bytes`, in file order, without decoding values.
pub fn list_messages(bytes: &[u8]) -> Result<Vec<MessageInfo>> {
    let file = parse_file(bytes)?;
    file.iter().map(|(_, sub)| info_of(&sub)).collect()
}

fn parse_file(
    bytes: &[u8],
) -> Result<grib::Grib2<grib::SeekableGrib2Reader<std::io::Cursor<&[u8]>>>> {
    grib::from_bytes(bytes).map_err(|e| GribError::Parse(e.to_string()))
}

/// Value of a first fixed surface with its scale factor applied, as pygrib `level` reports it.
///
/// Scale factor octet 0xFF is "missing" and eccodes then uses scale 0. The `grib` crate reads that
/// octet as sign-magnitude -127, so -127 is mapped to 0 here.
fn level_of(surface: &grib::FixedSurface) -> f64 {
    let scale = if surface.scale_factor == -127 {
        0
    } else {
        i32::from(surface.scale_factor)
    };
    f64::from(surface.scaled_value) * 10f64.powi(-scale)
}

fn info_of<R>(sub: &SubMessage<'_, R>) -> Result<MessageInfo> {
    let prod = sub.prod_def();
    let (surface_type, level_value) = match prod.fixed_surfaces() {
        Some((first, _)) => (Some(first.surface_type), Some(level_of(&first))),
        None => (None, None),
    };
    let sec3 = sub
        .section3()
        .map_err(|e| GribError::Parse(format!("section 3: {e}")))?;
    let (grid_template, resolution_flags) = match &sec3.payload.template {
        GridDefinitionTemplate::_3_0(t) => (0, t.lat_lon.grid.resolution_and_component_flags.0),
        GridDefinitionTemplate::_3_30(t) => (30, t.resolution_and_component_flags.0),
        _ => (sec3.payload.template_num, 0),
    };
    let sec5 = sub
        .section5()
        .map_err(|e| GribError::Parse(format!("section 5: {e}")))?;
    Ok(MessageInfo {
        discipline: sub.indicator().discipline,
        category: prod.parameter_category(),
        number: prod.parameter_number(),
        surface_type,
        level_value,
        grid_template,
        data_template: sec5.payload.template_num,
        resolution_flags,
    })
}

fn decode_submessage<R: Grib2Read>(
    bytes: &[u8],
    sub: SubMessage<'_, R>,
    info: MessageInfo,
) -> Result<DecodedMessage> {
    let sec3 = sub
        .section3()
        .map_err(|e| GribError::Parse(format!("section 3: {e}")))?;
    let template = sec3.payload.template.clone();
    let grid = match &template {
        GridDefinitionTemplate::_3_0(t) => {
            let (ni, nj, lats, lons) = regular_axes(t)?;
            let values = decode_values(bytes, sub, ni * nj)?;
            Grid {
                width: ni,
                height: nj,
                values,
                geo: GridGeo::Regular { lats, lons },
            }
        }
        GridDefinitionTemplate::_3_30(t) => {
            let (ni, nj) = (t.ni as usize, t.nj as usize);
            let latlons = lambert_latlons(&sub, ni * nj)?;
            let values = decode_values(bytes, sub, ni * nj)?;
            lambert_grid(t, ni, nj, values, &latlons)?
        }
        _ => {
            return Err(GribError::Unsupported(format!(
                "grid definition template 3.{}",
                sec3.payload.template_num
            )));
        }
    };
    Ok(DecodedMessage { grid, info })
}

/// Decodes the data values of a submessage (bitmap applied; missing = NaN).
///
/// Templates 5.0, 5.2, 5.3 and 5.41 are unpacked here in f64 (see `packing`), so the result is
/// bit-identical to eccodes after the f32 cast. Other templates use the `grib` crate's values.
fn decode_values<R: Grib2Read>(
    bytes: &[u8],
    sub: SubMessage<'_, R>,
    expected: usize,
) -> Result<Vec<f32>> {
    if let Some(values) = exact_values(bytes, &sub, expected)? {
        return Ok(values);
    }
    let decoder = Grib2SubmessageDecoder::from(sub)
        .map_err(|e| GribError::Decode(format!("decoder setup: {e}")))?;
    let iter = decoder
        .dispatch()
        .map_err(|e| GribError::Decode(format!("data representation: {e}")))?;
    let values: Vec<f32> = iter.collect();
    if values.len() != expected {
        return Err(GribError::Decode(format!(
            "decoded {} values, grid has {expected} points",
            values.len()
        )));
    }
    Ok(values)
}

/// Exact f64 unpacking for templates 5.0, 5.2, 5.3 and 5.41. `Ok(None)` for other templates.
fn exact_values<R>(
    bytes: &[u8],
    sub: &SubMessage<'_, R>,
    expected: usize,
) -> Result<Option<Vec<f32>>> {
    let s5 = section_bytes(bytes, sub.5.body)?;
    let template = u16::from_be_bytes([s5[9], s5[10]]);
    if !matches!(template, 0 | 2 | 3 | 41) {
        return Ok(None);
    }
    let s6 = section_bytes(bytes, sub.6.body)?;
    let s7 = section_bytes(bytes, sub.7.body)?;
    let payload = s7
        .get(5..)
        .ok_or_else(|| GribError::Parse("section 7 shorter than its header".into()))?;
    let n_encoded = u32::from_be_bytes([s5[5], s5[6], s5[7], s5[8]]) as usize;
    let scale = Scale::from_section5(s5)?;
    let (binary, decimal) = scale.factors();
    let ints: Vec<Option<i64>> = match template {
        0 | 41 => {
            let raw = if template == 0 {
                unpack_simple(payload, scale.nbits, n_encoded)?
            } else {
                unpack_png(payload, scale.nbits, n_encoded)?
            };
            raw.into_iter().map(Some).collect()
        }
        _ => {
            let c = Complex::from_section5(s5, template == 3)?;
            unpack_complex(&c, scale.nbits, payload, n_encoded)?
        }
    };
    let packed: Vec<f64> = ints
        .into_iter()
        .map(|v| v.map_or(f64::NAN, |x| scale.apply(x, binary, decimal)))
        .collect();
    expand_bitmap(packed, Some(s6), expected).map(Some)
}

/// Full bytes of one section, bounds-checked.
fn section_bytes<'a>(bytes: &'a [u8], info: &grib::SectionInfo) -> Result<&'a [u8]> {
    bytes
        .get(info.offset..info.offset + info.size)
        .ok_or_else(|| GribError::Parse(format!("section {} lies outside the message", info.num)))
}

/// Converts stored micro-degree integers to degrees (Lambert template 3.30 parameters).
#[inline]
fn micro_deg(v: i32) -> f64 {
    f64::from(v) / 1e6
}

#[inline]
fn micro_deg_u(v: u32) -> f64 {
    f64::from(v) / 1e6
}

/// Degrees for a stored lat/lon integer under template 3.0's angle unit.
///
/// WMO note 1 to template 3.0 (and eccodes' `angleDivisor`/`angleMultiplier`): when the basic angle
/// is 0 or missing (all ones), or the subdivisions are 0 or missing, the unit is 10^-6 degree.
/// Otherwise it is `basic_angle / subdivisions` degrees, evaluated as eccodes does
/// (`value / subdivisions * basic_angle`). MRMS sets basic angle 1 with 1,000,000 subdivisions.
#[inline]
pub(crate) fn grid_deg(v: f64, basic_angle: u32, subdivisions: u32) -> f64 {
    if basic_angle == 0 || basic_angle == u32::MAX || subdivisions == 0 || subdivisions == u32::MAX
    {
        v / 1e6
    } else {
        v / f64::from(subdivisions) * f64::from(basic_angle)
    }
}

/// Python: `lons = np.where(lons > 180.0, lons - 360.0, lons)`.
#[inline]
pub(crate) fn normalize_lon(lon: f64) -> f64 {
    if lon > 180.0 { lon - 360.0 } else { lon }
}

/// Regular lat/lon axes from the header, as `_grid_axes` does for `regular_ll`:
/// `linspace(first, last, Nj)` and `linspace(first, last, Ni)`, lons normalised.
fn regular_axes(
    t: &grib::def::grib2::template::Template3_0,
) -> Result<(usize, usize, Vec<f64>, Vec<f64>)> {
    let g = &t.lat_lon.grid;
    let (basic, sub) = (
        g.initial_production_domain_basic_angle,
        g.basic_angle_subdivisions,
    );
    let scan = t.lat_lon.scanning_mode.0;
    if scan & 0x80 != 0 || scan & 0x20 != 0 {
        return Err(GribError::Unsupported(format!(
            "regular_ll scanning mode {scan:#04x} (i negative or j consecutive)"
        )));
    }
    let (ni, nj) = (g.ni as usize, g.nj as usize);
    let lats = linspace(
        grid_deg(f64::from(g.first_point_lat), basic, sub),
        grid_deg(f64::from(g.last_point_lat), basic, sub),
        nj,
    );
    let lons: Vec<f64> = linspace(
        grid_deg(f64::from(g.first_point_lon), basic, sub),
        grid_deg(f64::from(g.last_point_lon), basic, sub),
        ni,
    )
    .into_iter()
    .map(normalize_lon)
    .collect();
    Ok((ni, nj, lats, lons))
}

/// Lat/lon of every grid point, in stored order, longitudes normalised like the Python.
fn lambert_latlons<R: Grib2Read>(
    sub: &SubMessage<'_, R>,
    expected: usize,
) -> Result<Vec<(f64, f64)>> {
    let iter = sub
        .latlons_unchecked()
        .map_err(|e| GribError::Decode(format!("lambert lat/lon: {e}")))?;
    let pts: Vec<(f64, f64)> = iter.map(|(lat, lon)| (lat, normalize_lon(lon))).collect();
    if pts.len() != expected {
        return Err(GribError::Decode(format!(
            "lambert lat/lon gave {} points, grid has {expected}",
            pts.len()
        )));
    }
    Ok(pts)
}

/// Builds a projected grid for a Lambert template (spec §5.3 `_extract_native_projection`).
///
/// The projected axes are the spec's regular axes: `x0` and `x1` are the nanmedians of the first and
/// last columns of projected x, `y0` and `y1` the nanmedians of the first and last rows of projected
/// y, and `linspace` builds the 1-D axes. If that procedure fails (non-finite medians or a span below
/// 1e-6 m), the grid falls back to 1-D lat/lon axes, as the Python does.
fn lambert_grid(
    t: &grib::def::grib2::template::Template3_30,
    ni: usize,
    nj: usize,
    values: Vec<f32>,
    latlons: &[(f64, f64)],
) -> Result<Grid> {
    let (a, b) = t.earth_shape.radii().ok_or_else(|| {
        GribError::Unsupported(format!("Earth shape code {}", t.earth_shape.shape))
    })?;
    if (a - b).abs() > 0.0 {
        return Err(GribError::Unsupported(
            "oblate Earth in Lambert grid (only spherical Earth is supported)".into(),
        ));
    }
    let crs = Crs::LambertConformal {
        lat_1: micro_deg(t.latin1),
        lat_2: micro_deg(t.latin2),
        lat_0: micro_deg(t.lad),
        lon_0: micro_deg_u(t.lov),
        radius_m: a,
    };
    let lat_mesh: Vec<f64> = latlons.iter().map(|p| p.0).collect();
    let lon_mesh: Vec<f64> = latlons.iter().map(|p| p.1).collect();

    let mut xs = vec![0.0_f64; ni * nj];
    let mut ys = vec![0.0_f64; ni * nj];
    for (k, (lat, lon)) in latlons.iter().enumerate() {
        let (x, y) = crs.forward(*lon, *lat);
        xs[k] = x;
        ys[k] = y;
    }
    let at = |r: usize, c: usize| r * ni + c;
    let col_first: Vec<f64> = (0..nj).map(|r| xs[at(r, 0)]).collect();
    let col_last: Vec<f64> = (0..nj).map(|r| xs[at(r, ni - 1)]).collect();
    let row_first: Vec<f64> = (0..ni).map(|c| ys[at(0, c)]).collect();
    let row_last: Vec<f64> = (0..ni).map(|c| ys[at(nj - 1, c)]).collect();
    let x0 = nanmedian(&col_first);
    let x1 = nanmedian(&col_last);
    let y0 = nanmedian(&row_first);
    let y1 = nanmedian(&row_last);
    let usable = [x0, x1, y0, y1].iter().all(|v| v.is_finite())
        && (x1 - x0).abs() >= 1e-6
        && (y1 - y0).abs() >= 1e-6;

    if usable {
        let x = linspace(x0, x1, ni);
        let y = linspace(y0, y1, nj);
        Ok(Grid {
            width: ni,
            height: nj,
            values,
            geo: GridGeo::Projected {
                crs,
                x,
                y,
                lat_mesh,
                lon_mesh,
            },
        })
    } else {
        // Python fallback: 1-D axes from column 0 (lats) and row 0 (lons).
        let lats: Vec<f64> = (0..nj).map(|r| lat_mesh[at(r, 0)]).collect();
        let lons: Vec<f64> = (0..ni).map(|c| lon_mesh[at(0, c)]).collect();
        Ok(Grid {
            width: ni,
            height: nj,
            values,
            geo: GridGeo::Regular { lats, lons },
        })
    }
}

/// numpy-compatible `linspace(start, stop, num)` with endpoint=True.
///
/// `step = (stop - start) / (num - 1)`, `y_i = i * step + start`, and the last element is set to
/// `stop` exactly. Matches numpy's operation order, so results are bit-identical in float64.
pub fn linspace(start: f64, stop: f64, num: usize) -> Vec<f64> {
    match num {
        0 => Vec::new(),
        1 => vec![start],
        _ => {
            let div = (num - 1) as f64;
            let delta = stop - start;
            let step = delta / div;
            let mut out: Vec<f64> = (0..num).map(|i| i as f64 * step + start).collect();
            out[num - 1] = stop;
            out
        }
    }
}

/// numpy-compatible `nanmedian` over a 1-D slice. Returns NaN when no finite value exists.
///
/// For an even count the result is `(a + b) / 2` of the two middle values, as `np.median` computes it.
pub fn nanmedian(values: &[f64]) -> f64 {
    let mut v: Vec<f64> = values.iter().copied().filter(|x| !x.is_nan()).collect();
    if v.is_empty() {
        return f64::NAN;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn template_3_0_angle_units() {
        // Basic angle 0 or missing: micro-degrees.
        assert_eq!(super::grid_deg(54_995_000.0, 0, 0), 54.995);
        assert_eq!(super::grid_deg(54_995_000.0, u32::MAX, u32::MAX), 54.995);
        assert_eq!(super::grid_deg(54_995_000.0, 0, 1_000_000), 54.995);
        // MRMS: basic angle 1, 10^6 subdivisions (same unit, explicit).
        assert_eq!(super::grid_deg(54_995_000.0, 1, 1_000_000), 54.995);
        assert_eq!(super::grid_deg(230_005_000.0, 1, 1_000_000), 230.005);
        // A coarser unit: basic angle 1, 1000 subdivisions (millidegrees).
        assert_eq!(super::grid_deg(54_995.0, 1, 1000), 54.995);
    }

    use super::*;

    #[test]
    fn linspace_matches_numpy_endpoints() {
        let v = linspace(54.0, 20.0, 35);
        assert_eq!(v.len(), 35);
        assert_eq!(v[0], 54.0);
        assert_eq!(v[34], 20.0);
        assert!((v[1] - 53.0).abs() < 1e-12);
        assert_eq!(linspace(1.0, 2.0, 1), vec![1.0]);
        assert!(linspace(1.0, 2.0, 0).is_empty());
    }

    #[test]
    fn nanmedian_odd_even_and_all_nan() {
        assert_eq!(nanmedian(&[3.0, f64::NAN, 1.0, 2.0]), 2.0);
        assert_eq!(nanmedian(&[1.0, 2.0, 3.0, 4.0]), 2.5);
        assert!(nanmedian(&[f64::NAN]).is_nan());
    }

    #[test]
    fn selector_matches_only_constrained_fields() {
        let info = MessageInfo {
            discipline: 0,
            category: Some(16),
            number: Some(196),
            surface_type: Some(10),
            level_value: Some(0.0),
            grid_template: 30,
            data_template: 3,
            resolution_flags: 0,
        };
        assert!(
            MessageSelector::parameter(0, 16, 196)
                .at_level(10, 0.0)
                .matches(&info)
        );
        assert!(!MessageSelector::parameter(0, 0, 0).matches(&info));
        assert!(
            !MessageSelector::parameter(0, 16, 196)
                .at_level(103, 2.0)
                .matches(&info)
        );
        assert!(MessageSelector::default().matches(&info));
    }

    #[test]
    fn lon_normalisation_matches_python() {
        assert_eq!(normalize_lon(230.0), -130.0);
        assert_eq!(normalize_lon(180.0), 180.0);
        assert_eq!(normalize_lon(-97.5), -97.5);
    }

    #[test]
    fn uv_flag_is_bit_three() {
        let mut info = MessageInfo {
            discipline: 0,
            category: None,
            number: None,
            surface_type: None,
            level_value: None,
            grid_template: 30,
            data_template: 3,
            resolution_flags: 8,
        };
        let grid = Grid {
            width: 1,
            height: 1,
            values: vec![0.0],
            geo: GridGeo::Regular {
                lats: vec![0.0],
                lons: vec![0.0],
            },
        };
        assert!(
            DecodedMessage {
                grid: grid.clone(),
                info
            }
            .uv_grid_relative()
        );
        info.resolution_flags = 48;
        assert!(!DecodedMessage { grid, info }.uv_grid_relative());
    }
}
