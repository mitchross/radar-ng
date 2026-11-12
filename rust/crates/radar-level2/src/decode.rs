use crate::{
    ARRAY_LIMIT, COMPRESSED_LIMIT, EXPANDED_LIMIT, GATE_LIMIT, RADIAL_LIMIT, RECORD_LIMIT,
    SWEEP_LIMIT,
};
use anyhow::{Context, Result, ensure};
use chrono::{DateTime, Utc};
use nexrad_decode::messages::{
    MessageContents,
    digital_radar_data::{DataBlock, GenericDataBlock},
};
use serde::{Deserialize, Serialize};
use std::io::Read;

pub const BELOW: u8 = 1;
pub const FOLDED: u8 = 2;
pub const BLANKED: u8 = 4;
pub const UNOBSERVED: u8 = 8;

#[derive(Debug, Serialize, Deserialize)]
pub struct Moment {
    pub first_gate_m: i16,
    pub spacing_m: i16,
    pub word_bits: u8,
    pub scale: f32,
    pub offset: f32,
    pub control_flags: u8,
    pub snr_threshold_raw: i16,
    pub tover_raw: u16,
    pub encoded: Vec<u8>,
    pub values: Vec<f32>,
    pub qc: Vec<u8>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Ray {
    pub number: u16,
    pub time: DateTime<Utc>,
    pub azimuth_deg: f32,
    pub elevation_deg: f32,
    pub azimuth_spacing_deg: f32,
    pub radial_status: u8,
    pub blanking: u8,
    pub moments: [Option<Moment>; 3],
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct VolumeSite {
    pub latitude_deg: f32,
    pub longitude_deg: f32,
    pub site_height_m_raw: i16,
    pub tower_height_m_raw: u16,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Sweep {
    pub station: String,
    pub volume_start: DateTime<Utc>,
    pub volume_number: String,
    pub ordinal: usize,
    pub elevation_number: u8,
    pub vcp: Option<u16>,
    pub site: Option<VolumeSite>,
    pub scan_start: DateTime<Utc>,
    pub scan_end: DateTime<Utc>,
    pub rays: Vec<Ray>,
    pub missing_sectors_deg: Vec<[f32; 2]>,
}

pub struct Decoder {
    station: String,
    volume_start: Option<DateTime<Utc>>,
    volume_number: String,
    compressed: usize,
    expanded: usize,
    owned: usize,
    ordinal: usize,
    current: Option<Sweep>,
    lowest: Option<f32>,
    vcp: Option<u16>,
    site: Option<VolumeSite>,
}

pub fn capped_read(reader: impl Read, limit: usize) -> Result<Vec<u8>> {
    let mut result = Vec::new();
    let mut reader = reader;
    let mut block = [0u8; 16384];
    loop {
        let room = limit - result.len();
        let n = reader.read(&mut block[..room.min(16384)])?;
        if n == 0 {
            if room == 0 {
                let mut extra = [0];
                ensure!(reader.read(&mut extra)? == 0, "expanded byte limit");
            }
            return Ok(result);
        }
        ensure!(result.len() + n <= limit, "expanded byte limit");
        result.try_reserve_exact(n)?;
        result.extend_from_slice(&block[..n]);
    }
}

impl Decoder {
    pub fn new(station: &str) -> Self {
        Self {
            station: station.into(),
            volume_start: None,
            volume_number: String::new(),
            compressed: 0,
            expanded: 0,
            owned: 0,
            ordinal: 0,
            current: None,
            lowest: None,
            vcp: None,
            site: None,
        }
    }

    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<Sweep>> {
        ensure!(
            self.compressed
                .checked_add(bytes.len())
                .is_some_and(|n| n <= COMPRESSED_LIMIT),
            "compressed volume limit"
        );
        self.compressed += bytes.len();
        let records = if self.volume_start.is_none() {
            ensure!(bytes.starts_with(b"AR2V"), "unsupported archive format");
            let file = nexrad_data::volume::File::new(
                bytes.get(..24).context("truncated header")?.to_vec(),
            );
            let h = file.header().context("invalid volume header")?;
            ensure!(
                h.icao_of_radar() == Some(self.station.as_str()),
                "station mismatch"
            );
            self.volume_start = Some(h.date_time().context("invalid volume time")?);
            self.volume_number = h
                .extension_number()
                .context("invalid volume number")?
                .into();
            &bytes[24..]
        } else {
            bytes
        };
        let mut completed = vec![];
        let mut pos = 0;
        while pos < records.len() {
            let raw: [u8; 4] = records
                .get(pos..pos + 4)
                .context("truncated record length")?
                .try_into()?;
            let length = i32::from_be_bytes(raw).unsigned_abs() as usize;
            ensure!(
                length > 0 && length <= COMPRESSED_LIMIT,
                "invalid record length"
            );
            let compressed = records
                .get(pos + 4..pos + 4 + length)
                .context("truncated record")?;
            ensure!(
                compressed.starts_with(b"BZh"),
                "unsupported uncompressed record"
            );
            let remaining = EXPANDED_LIMIT
                .checked_sub(self.expanded)
                .context("expanded volume limit")?;
            let mut bz = bzip2::read::BzDecoder::new(compressed);
            let expanded = capped_read(&mut bz, RECORD_LIMIT.min(remaining))?;
            ensure!(
                bz.total_in() as usize == compressed.len(),
                "trailing compressed record bytes"
            );
            self.expanded += expanded.len();
            for message in nexrad_decode::iter_messages(&expanded) {
                let message = message?;
                match message.contents() {
                    MessageContents::DigitalRadarData(m) => {
                        if let Some(sweep) = self.radial(m)? {
                            completed.push(sweep);
                        }
                    }
                    MessageContents::DigitalRadarDataLegacy(_) => {
                        anyhow::bail!("legacy Message 1 unsupported")
                    }
                    _ => {}
                }
            }
            pos += 4 + length;
        }
        Ok(completed)
    }

    fn radial(
        &mut self,
        m: &nexrad_decode::messages::digital_radar_data::Message<'_>,
    ) -> Result<Option<Sweep>> {
        let h = m.header();
        ensure!(
            h.radar_identifier() == Some(self.station.as_str()),
            "radial station mismatch"
        );
        ensure!(
            h.compression_indicator_raw() == 0,
            "compressed radial unsupported"
        );
        let status = h.radial_status_raw();
        ensure!(status <= 5, "unknown radial status");
        let time = h.date_time().context("invalid radial time")?;
        ensure!(h.time_raw() < 86400000, "invalid radial clock");
        let az = h.azimuth_angle_degrees();
        let el = h.elevation_angle_degrees();
        let spacing = h.azimuth_resolution_spacing_degrees();
        ensure!(
            az.is_finite()
                && (0.0..360.0).contains(&az)
                && el.is_finite()
                && (-5.0..=90.0).contains(&el)
                && [0.5, 1.0].contains(&spacing),
            "invalid ray geometry"
        );
        if let Some(v) = m.volume_data_block() {
            self.vcp = Some(v.inner().vcp_number_raw());
            let site = VolumeSite {
                latitude_deg: v.inner().latitude_degrees(),
                longitude_deg: v.inner().longitude_degrees(),
                site_height_m_raw: v.inner().site_height_meters(),
                tower_height_m_raw: v.inner().tower_height_meters(),
            };
            ensure!(
                site.latitude_deg.is_finite()
                    && (-90.0..=90.0).contains(&site.latitude_deg)
                    && site.longitude_deg.is_finite()
                    && (-180.0..=180.0).contains(&site.longitude_deg),
                "invalid volume site geometry"
            );
            self.site = Some(site);
        }
        if [0, 3, 5].contains(&status) {
            ensure!(self.current.is_none(), "previous sweep incomplete");
            self.ordinal += 1;
            ensure!(self.ordinal <= SWEEP_LIMIT, "sweep count limit");
            ensure!(h.azimuth_number() == 1, "sweep missing first radial");
            self.current = Some(Sweep {
                station: self.station.clone(),
                volume_start: self.volume_start.unwrap(),
                volume_number: self.volume_number.clone(),
                ordinal: self.ordinal,
                elevation_number: h.elevation_number(),
                vcp: self.vcp,
                site: self.site,
                scan_start: time,
                scan_end: time,
                rays: vec![],
                missing_sectors_deg: vec![],
            });
        }
        let s = self
            .current
            .as_ref()
            .context("radial without sweep start")?;
        ensure!(s.rays.len() < RADIAL_LIMIT, "radial count limit");
        ensure!(
            s.elevation_number == h.elevation_number()
                && h.azimuth_number() as usize == s.rays.len() + 1
                && time >= s.scan_end,
            "radial discontinuity"
        );
        let blocks = [
            m.reflectivity_data_block(),
            m.correlation_coefficient_data_block(),
            m.differential_reflectivity_data_block(),
        ];
        // Headers borrow the capped record; validate every moment before allocating retained arrays.
        for block in blocks
            .iter()
            .copied()
            .flatten()
            .chain(
                [
                    m.velocity_data_block(),
                    m.spectrum_width_data_block(),
                    m.differential_phase_data_block(),
                    m.clutter_filter_power_data_block(),
                ]
                .into_iter()
                .flatten(),
            )
            .chain(m.other_data_blocks().iter())
        {
            validate_moment(block)?;
        }
        let required: usize = blocks
            .iter()
            .flatten()
            .map(|b| {
                b.inner().encoded_values().len()
                    + usize::from(b.inner().header().data_moment_gate_count()) * 5
            })
            .sum::<usize>()
            + std::mem::size_of::<Ray>();
        ensure!(
            self.owned
                .checked_add(required)
                .is_some_and(|n| n <= ARRAY_LIMIT),
            "decoded byte limit"
        );
        self.owned += required;
        let moments = [
            blocks[0].map(|b| moment(b, h.radial_spot_blanking_status_raw())),
            blocks[1].map(|b| moment(b, h.radial_spot_blanking_status_raw())),
            blocks[2].map(|b| moment(b, h.radial_spot_blanking_status_raw())),
        ];
        let [a, b, c] = moments;
        let s = self.current.as_mut().unwrap();
        s.rays.push(Ray {
            number: h.azimuth_number(),
            time,
            azimuth_deg: az,
            elevation_deg: el,
            azimuth_spacing_deg: spacing,
            radial_status: status,
            blanking: h.radial_spot_blanking_status_raw(),
            moments: [a.transpose()?, b.transpose()?, c.transpose()?],
        });
        s.scan_end = time;
        if [2, 4].contains(&status) {
            let mut s = self.current.take().unwrap();
            ensure!(
                s.rays.len() == (360.0 / spacing) as usize,
                "incomplete azimuth coverage"
            );
            let mut rotation = 0.0;
            for (ray, next) in s.rays.iter().zip(s.rays.iter().cycle().skip(1)) {
                ensure!(ray.azimuth_spacing_deg == spacing, "mixed azimuth spacing");
                let gap =
                    (f64::from(next.azimuth_deg) - f64::from(ray.azimuth_deg)).rem_euclid(360.0);
                ensure!(
                    gap >= f64::from(spacing) * 0.25 && gap < 180.0,
                    "duplicate or reversed azimuth"
                );
                rotation += gap;
                if gap > f64::from(spacing) * 1.8 {
                    s.missing_sectors_deg
                        .push([ray.azimuth_deg, next.azimuth_deg]);
                }
            }
            ensure!((rotation - 360.0).abs() < 0.001, "invalid azimuth rotation");
            let lowest = *self.lowest.get_or_insert(el);
            ensure!(el >= lowest - 0.15, "unexpected lower sweep");
            if s.rays.iter().any(|r| r.moments[0].is_some()) && (el - lowest).abs() <= 0.15 {
                return Ok(Some(s));
            }
            self.owned -= s
                .rays
                .iter()
                .map(|r| {
                    std::mem::size_of::<Ray>()
                        + r.moments
                            .iter()
                            .flatten()
                            .map(|m| m.encoded.len() + m.values.len() * 5)
                            .sum::<usize>()
                })
                .sum::<usize>();
        }
        Ok(None)
    }

    pub fn finish(&self) -> Result<()> {
        ensure!(self.current.is_none(), "incomplete sweep at EOF");
        Ok(())
    }
}

fn validate_moment(b: &DataBlock<'_, GenericDataBlock<'_>>) -> Result<()> {
    let h = b.inner().header();
    ensure!(
        usize::from(h.data_moment_gate_count()) <= GATE_LIMIT,
        "gate count limit"
    );
    ensure!(
        [8, 16].contains(&h.data_word_size_raw()),
        "invalid moment word size"
    );
    ensure!(
        h.scale().is_finite() && h.scale() > 0.0 && h.offset().is_finite(),
        "invalid moment encoding"
    );
    ensure!(
        (h.data_moment_range_sample_interval_raw() as i16) > 0,
        "invalid gate spacing"
    );
    ensure!(
        b.inner().encoded_values().len()
            == usize::from(h.data_moment_gate_count()) * usize::from(h.data_word_size_raw() / 8),
        "truncated moment"
    );
    Ok(())
}

fn moment(b: &DataBlock<'_, GenericDataBlock<'_>>, blanking: u8) -> Result<Moment> {
    let h = b.inner().header();
    let encoded = b.inner().encoded_values().to_vec();
    let qc_started = std::time::Instant::now();
    let (values, qc) = qc_values(
        &encoded,
        h.data_word_size_raw(),
        h.scale(),
        h.offset(),
        blanking,
    )?;
    crate::metrics::add(
        "radar_level2_qc_seconds_total",
        &[],
        qc_started.elapsed().as_secs_f64(),
    );
    Ok(Moment {
        first_gate_m: h.data_moment_range_raw() as i16,
        spacing_m: h.data_moment_range_sample_interval_raw() as i16,
        word_bits: h.data_word_size_raw(),
        scale: h.scale(),
        offset: h.offset(),
        control_flags: h.control_flags_raw(),
        snr_threshold_raw: h.snr_threshold_raw(),
        tover_raw: h.tover_raw(),
        encoded,
        values,
        qc,
    })
}

pub fn qc_values(
    raw: &[u8],
    bits: u8,
    scale: f32,
    offset: f32,
    blanking: u8,
) -> Result<(Vec<f32>, Vec<u8>)> {
    ensure!(
        [8, 16].contains(&bits) && raw.len().is_multiple_of(usize::from(bits / 8)),
        "invalid encoded moment"
    );
    let n = raw.len() / usize::from(bits / 8);
    ensure!(
        n <= GATE_LIMIT && scale.is_finite() && scale > 0.0 && offset.is_finite(),
        "invalid moment bounds"
    );
    let mut values = Vec::with_capacity(n);
    let mut qc = Vec::with_capacity(n);
    for bytes in raw.chunks_exact(usize::from(bits / 8)) {
        let raw = if bits == 8 {
            u16::from(bytes[0])
        } else {
            u16::from_be_bytes([bytes[0], bytes[1]])
        };
        let flags = match raw {
            0 => BELOW,
            1 => FOLDED,
            _ => 0,
        } | if blanking != 0 { BLANKED } else { 0 };
        let value = (f32::from(raw) - offset) / scale;
        ensure!(value.is_finite(), "nonfinite moment");
        values.push(if flags == 0 { value } else { f32::NAN });
        qc.push(flags);
    }
    Ok((values, qc))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn conservative_values_and_statuses() {
        let (v, q) = qc_values(&[0, 1, 2, 40, 200, 255], 8, 2.0, 66.0, 0).unwrap();
        assert!(v[0].is_nan() && v[1].is_nan());
        assert_eq!(q, [BELOW, FOLDED, 0, 0, 0, 0]);
        assert_eq!(&v[2..], &[-32.0, -13.0, 67.0, 94.5]);
        let (v, q) = qc_values(&[0, 0, 0, 1, 0x01, 0x00], 16, 2.0, 66.0, 1).unwrap();
        assert!(v.iter().all(|v| v.is_nan()));
        assert_eq!(q, [BELOW | BLANKED, FOLDED | BLANKED, BLANKED]);
        assert!(qc_values(&vec![2; GATE_LIMIT + 1], 8, 1.0, 0.0, 0).is_err());
        assert!(qc_values(&[2], 8, 0.0, 0.0, 0).is_err());
    }
    #[test]
    fn capped_expansion_including_exact_boundary() {
        assert_eq!(capped_read(&b"abcd"[..], 4).unwrap(), b"abcd");
        assert!(capped_read(&b"abcde"[..], 4).is_err());
        let mut out = vec![];
        let mut enc = bzip2::write::BzEncoder::new(&mut out, bzip2::Compression::best());
        use std::io::Write;
        enc.write_all(&vec![0; RECORD_LIMIT + 1]).unwrap();
        enc.finish().unwrap();
        assert!(capped_read(bzip2::read::BzDecoder::new(out.as_slice()), RECORD_LIMIT).is_err());
    }
    #[test]
    fn compressed_and_truncated_caps() {
        let mut d = Decoder::new("KGRR");
        assert!(d.push(&vec![0; COMPRESSED_LIMIT + 1]).is_err());
        assert!(Decoder::new("KGRR").push(b"AR2V").is_err());
    }
    #[test]
    fn compressed_and_expanded_volume_limits_are_inclusive() {
        use std::io::Write;
        let mut header = b"AR2V0006.001".to_vec();
        header.extend_from_slice(&20765u32.to_be_bytes());
        header.extend_from_slice(&0u32.to_be_bytes());
        header.extend_from_slice(b"KGRR");
        let mut d = Decoder::new("KGRR");
        d.compressed = COMPRESSED_LIMIT - header.len();
        assert!(d.push(&header).unwrap().is_empty());
        assert!(
            d.push(&[0])
                .unwrap_err()
                .to_string()
                .contains("compressed volume limit")
        );
        let mut bz = bzip2::write::BzEncoder::new(Vec::new(), bzip2::Compression::fast());
        bz.write_all(&[0; 16]).unwrap();
        let bz = bz.finish().unwrap();
        let mut record = (bz.len() as i32).to_be_bytes().to_vec();
        record.extend(bz);
        let mut d = Decoder::new("KGRR");
        d.push(&header).unwrap();
        d.expanded = EXPANDED_LIMIT - 16;
        assert!(d.push(&record).unwrap().is_empty());
        assert_eq!(d.expanded, EXPANDED_LIMIT);
        assert!(
            d.push(&record)
                .unwrap_err()
                .to_string()
                .contains("expanded byte limit")
        );
    }
}
