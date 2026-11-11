use crate::{decode::Sweep, feed::Source};
use anyhow::{Context, Result, ensure};
use futures_util::StreamExt;
use object_store::{ObjectStore, PutMode, path::Path};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, Serialize, Deserialize)]
pub struct Entry {
    pub key: String,
    pub sha256: String,
    pub bytes: usize,
}
#[derive(Debug, Serialize, Deserialize)]
pub struct Commit {
    pub schema_version: u8,
    pub identity: String,
    pub station: String,
    pub scan_end: chrono::DateTime<chrono::Utc>,
    pub sources: Vec<Source>,
    pub entries: Vec<Entry>,
}

pub struct Artifact {
    pub commit: Commit,
    pub commit_key: String,
    objects: Vec<(String, Vec<u8>)>,
}

impl Artifact {
    pub fn build(sweep: &Sweep, sources: Vec<Source>, namespace: &str) -> Result<Self> {
        ensure!(
            namespace.starts_with("level2/")
                && namespace.split('/').all(|p| !p.is_empty()
                    && p.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')),
            "invalid Level II object namespace"
        );
        ensure!(
            sweep.rays.len() <= crate::RADIAL_LIMIT
                && sweep
                    .rays
                    .first()
                    .is_some_and(|r| [0, 3, 5].contains(&r.radial_status))
                && sweep
                    .rays
                    .last()
                    .is_some_and(|r| [2, 4].contains(&r.radial_status)),
            "incomplete artifact sweep"
        );
        let mut retained = 0usize;
        for ray in &sweep.rays {
            for m in ray.moments.iter().flatten() {
                ensure!(
                    m.values.len() <= crate::GATE_LIMIT && m.values.len() == m.qc.len(),
                    "artifact moment bounds"
                );
                retained = retained
                    .checked_add(m.encoded.len() + m.values.len() * 5)
                    .context("artifact byte overflow")?;
            }
        }
        ensure!(retained <= crate::ARRAY_LIMIT, "artifact memory limit");
        let geometry: Vec<_> = sweep
            .rays
            .iter()
            .map(|r| {
                json!([
                    r.azimuth_deg,
                    r.elevation_deg,
                    r.azimuth_spacing_deg,
                    r.moments
                        .iter()
                        .map(|m| m
                            .as_ref()
                            .map(|m| (m.first_gate_m, m.spacing_m, m.values.len())))
                        .collect::<Vec<_>>()
                ])
            })
            .collect();
        let geometry_digest = crate::checksum(&serde_json::to_vec(
            &json!({"rays":geometry,"site":sweep.site}),
        )?);
        let identity = crate::checksum(&serde_json::to_vec(
            &json!({"station":sweep.station,"volume_start":sweep.volume_start,"volume":sweep.volume_number,"ordinal":sweep.ordinal,"scan_start":sweep.scan_start,"scan_end":sweep.scan_end,"field":"REF","geometry":geometry_digest,"decoder":"nexrad-1.0.0","qc":"level2-conservative-v1","sources":sources}),
        )?);
        let prefix = format!(
            "{namespace}/sweeps/{}/{identity}/{}",
            sweep.station,
            uuid::Uuid::new_v4().simple()
        );
        let mut objects = vec![];
        objects.push((
            format!("{prefix}/.zgroup"),
            serde_json::to_vec(&json!({"zarr_format":2}))?,
        ));
        let mut metadata = json!({"station":sweep.station,"volume_start":sweep.volume_start,"volume_number":sweep.volume_number,"ordinal":sweep.ordinal,"elevation_number":sweep.elevation_number,"vcp":sweep.vcp,"site":sweep.site,"scan_start":sweep.scan_start,"scan_end":sweep.scan_end,"missing_sectors_deg":sweep.missing_sectors_deg});
        metadata["geometry_digest"] = json!(geometry_digest);
        metadata["decoder"] = json!("nexrad-1.0.0");
        metadata["qc_profile"] = json!("level2-conservative-v1");
        metadata["moments"] = json!(["REF", "RHO", "ZDR"]);
        metadata["antenna_altitude_m_msl"] = serde_json::Value::Null;
        metadata["rays"]=json!(sweep.rays.iter().map(|r|json!({"number":r.number,"azimuth_spacing_deg":r.azimuth_spacing_deg,"radial_status":r.radial_status,"blanking":r.blanking,"moments":r.moments.iter().map(|m|m.as_ref().map(|m|json!({"first_gate_m":m.first_gate_m,"spacing_m":m.spacing_m,"gates":m.values.len(),"word_bits":m.word_bits,"scale":m.scale,"offset":m.offset,"control_flags":m.control_flags,"snr_threshold_raw":m.snr_threshold_raw,"tover_raw":m.tover_raw}))).collect::<Vec<_>>()})).collect::<Vec<_>>());
        objects.push((format!("{prefix}/.zattrs"), serde_json::to_vec(&metadata)?));
        for (field, bytes, dtype) in [
            (
                "time_ms",
                sweep
                    .rays
                    .iter()
                    .flat_map(|r| r.time.timestamp_millis().to_le_bytes())
                    .collect::<Vec<_>>(),
                "<i8",
            ),
            (
                "azimuth_deg",
                sweep
                    .rays
                    .iter()
                    .flat_map(|r| r.azimuth_deg.to_le_bytes())
                    .collect(),
                "<f4",
            ),
            (
                "elevation_deg",
                sweep
                    .rays
                    .iter()
                    .flat_map(|r| r.elevation_deg.to_le_bytes())
                    .collect(),
                "<f4",
            ),
        ] {
            array(
                &mut objects,
                &prefix,
                field,
                &[sweep.rays.len()],
                dtype,
                bytes,
            )?;
        }
        for (index, name) in ["REF", "RHO", "ZDR"].iter().enumerate() {
            let moments = || sweep.rays.iter().filter_map(|r| r.moments[index].as_ref());
            let gates: usize = moments().map(|m| m.values.len()).sum();
            let encoded_bytes: usize = moments().map(|m| m.encoded.len()).sum();
            let mut values = Vec::with_capacity(gates * 4);
            let mut qc = Vec::with_capacity(gates);
            let mut encoded = Vec::with_capacity(encoded_bytes);
            let mut offsets = Vec::with_capacity(sweep.rays.len() + 1);
            offsets.push(0u64);
            for ray in &sweep.rays {
                if let Some(m) = &ray.moments[index] {
                    values.extend(m.values.iter().flat_map(|v| v.to_le_bytes()));
                    qc.extend_from_slice(&m.qc);
                    encoded.extend_from_slice(&m.encoded);
                }
                offsets.push(qc.len() as u64);
            }
            let len = qc.len();
            if len == 0 {
                continue;
            }
            array(&mut objects, &prefix, name, &[len], "<f4", values)?;
            array(
                &mut objects,
                &prefix,
                &format!("{name}_qc"),
                &[len],
                "|u1",
                qc,
            )?;
            array(
                &mut objects,
                &prefix,
                &format!("{name}_encoded"),
                &[encoded.len()],
                "|u1",
                encoded,
            )?;
            array(
                &mut objects,
                &prefix,
                &format!("{name}_offsets"),
                &[offsets.len()],
                "<u8",
                offsets.iter().flat_map(|n| n.to_le_bytes()).collect(),
            )?;
        }
        objects.sort_by(|a, b| a.0.cmp(&b.0));
        objects.dedup_by(|a, b| a.0 == b.0);
        let entries = objects
            .iter()
            .map(|(key, bytes)| Entry {
                key: key.clone(),
                sha256: crate::checksum(bytes),
                bytes: bytes.len(),
            })
            .collect();
        let commit = Commit {
            schema_version: 1,
            identity,
            station: sweep.station.clone(),
            scan_end: sweep.scan_end,
            sources,
            entries,
        };
        Ok(Self {
            commit,
            commit_key: format!("{prefix}/commit.json"),
            objects,
        })
    }
    pub fn bytes(&self) -> Result<i64> {
        Ok((self.objects.iter().map(|(_, b)| b.len()).sum::<usize>()
            + serde_json::to_vec(&self.commit)?.len()) as i64)
    }
    pub async fn publish(&self, store: &dyn ObjectStore) -> Result<()> {
        for (key, bytes) in &self.objects {
            write_verified(store, key, bytes).await?;
        }
        write_verified(store, &self.commit_key, &serde_json::to_vec(&self.commit)?).await?;
        verify(store, &self.commit_key).await?;
        Ok(())
    }
}

fn array(
    objects: &mut Vec<(String, Vec<u8>)>,
    prefix: &str,
    name: &str,
    shape: &[usize],
    dtype: &str,
    bytes: Vec<u8>,
) -> Result<()> {
    let width = match dtype {
        "<f4" => 4,
        "<i8" | "<u8" => 8,
        _ => 1,
    };
    let chunk = shape[0].clamp(1, 65536);
    objects.push((format!("{prefix}/{name}/.zarray"),serde_json::to_vec(&json!({"zarr_format":2,"shape":shape,"chunks":[chunk],"dtype":dtype,"compressor":null,"fill_value":null,"order":"C","filters":null}))?));
    for (i, bytes) in bytes.chunks(chunk * width).enumerate() {
        let mut padded = bytes.to_vec();
        padded.resize(chunk * width, 0);
        objects.push((format!("{prefix}/{name}/{i}"), padded));
    }
    Ok(())
}
pub(crate) async fn write_verified(store: &dyn ObjectStore, key: &str, bytes: &[u8]) -> Result<()> {
    let path = Path::from(key);
    store
        .put_opts(&path, bytes.to_vec().into(), PutMode::Create.into())
        .await?;
    let read = read_bounded(store, &path, bytes.len()).await?;
    ensure!(
        crate::checksum(&read) == crate::checksum(bytes),
        "artifact read-back checksum"
    );
    Ok(())
}
pub async fn verify(store: &dyn ObjectStore, key: &str) -> Result<Commit> {
    Ok(verify_collect(store, key, |_| false).await?.0)
}

/// Bytes the tile sampler retains from an artifact: REF, REF QC, offsets, ray angles, metadata.
fn wanted(relative: &str) -> bool {
    relative == ".zattrs"
        || ["azimuth_deg/", "elevation_deg/", "REF/", "REF_qc/", "REF_offsets/"]
            .iter()
            .any(|p| relative.starts_with(p))
}

pub async fn read_commit(store: &dyn ObjectStore, key: &str) -> Result<Commit> {
    Ok(serde_json::from_slice(
        &read_bounded(store, &Path::from(key), 2 * 1024 * 1024).await?,
    )?)
}

/// Raw bytes `load` will hold at once for this commit.
pub fn wanted_bytes(commit: &Commit) -> usize {
    commit
        .entries
        .iter()
        .filter(|e| e.key.rsplit_once("/sweeps/").is_some_and(|(_, r)| r.splitn(4, '/').nth(3).is_some_and(wanted)))
        .map(|e| e.bytes)
        .sum()
}

/// Verifies every object, then rebuilds the REF polar sweep the tile sampler needs.
pub async fn load(store: &dyn ObjectStore, key: &str) -> Result<(Commit, crate::polar::PolarSweep)> {
    let (commit, kept) = verify_collect(store, key, wanted).await?;
    let get = |name: &str| kept.get(name).with_context(|| format!("missing {name}"));
    let attributes: serde_json::Value = serde_json::from_slice(get(".zattrs")?)?;
    let site = &attributes["site"];
    let site = (
        site["latitude_deg"].as_f64().context("missing site latitude")?,
        site["longitude_deg"].as_f64().context("missing site longitude")?,
    );
    let rays = attributes["rays"].as_array().context("missing rays")?;
    let join = |name: &str, shape: usize, width: usize| -> Result<Vec<u8>> {
        let mut bytes = Vec::with_capacity(shape * width);
        let mut index = 0;
        while bytes.len() < shape * width {
            bytes.extend_from_slice(get(&format!("{name}/{index}"))?);
            index += 1;
        }
        bytes.truncate(shape * width);
        Ok(bytes)
    };
    let f32s = |b: Vec<u8>| -> Vec<f32> { b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect() };
    let azimuth = f32s(join("azimuth_deg", rays.len(), 4)?);
    let elevation = f32s(join("elevation_deg", rays.len(), 4)?);
    let offsets: Vec<u64> = join("REF_offsets", rays.len() + 1, 8)?
        .chunks_exact(8)
        .map(|c| u64::from_le_bytes(c.try_into().expect("8 bytes")))
        .collect();
    let gates = *offsets.last().context("missing offsets")? as usize;
    let values = f32s(join("REF", gates, 4)?);
    let qc = join("REF_qc", gates, 1)?;
    let mut inputs = Vec::with_capacity(rays.len());
    for (i, ray) in rays.iter().enumerate() {
        let m = &ray["moments"][0];
        let count = (offsets[i + 1].checked_sub(offsets[i])).context("offset order")? as usize;
        let field = |name: &str| m[name].as_i64().map(|v| v as i16);
        ensure!(m.is_null() == (count == 0), "ray gate count mismatch");
        inputs.push(crate::polar::RayInput {
            azimuth_deg: azimuth[i],
            elevation_deg: elevation[i],
            spacing_deg: ray["azimuth_spacing_deg"].as_f64().context("missing spacing")? as f32,
            first_gate_m: field("first_gate_m").unwrap_or(0),
            gate_m: field("spacing_m").unwrap_or(0),
            gates: count,
        });
    }
    let sweep = crate::polar::PolarSweep::new(commit.identity.clone(), site, &inputs, values, qc)?;
    Ok((commit, sweep))
}

async fn verify_collect(
    store: &dyn ObjectStore,
    key: &str,
    keep: impl Fn(&str) -> bool,
) -> Result<(Commit, std::collections::BTreeMap<String, Vec<u8>>)> {
    let mut kept = std::collections::BTreeMap::new();
    let commit: Commit = read_commit(store, key).await?;
    ensure!(
        commit.schema_version == 1 && !commit.entries.is_empty() && commit.entries.len() <= 4096,
        "invalid numeric commit"
    );
    let prefix = key
        .strip_suffix("commit.json")
        .context("invalid commit key")?;
    let mut inventory = std::collections::BTreeMap::new();
    let mut total = 0usize;
    for entry in &commit.entries {
        let relative = entry
            .key
            .strip_prefix(prefix)
            .context("artifact outside revision")?;
        ensure!(
            inventory.insert(relative, entry).is_none(),
            "duplicate artifact object"
        );
        total = total
            .checked_add(entry.bytes)
            .context("artifact byte overflow")?;
    }
    ensure!(total <= crate::JOB_RESERVATION, "artifact total byte limit");
    for required in [
        ".zgroup",
        ".zattrs",
        "time_ms/.zarray",
        "azimuth_deg/.zarray",
        "elevation_deg/.zarray",
        "REF/.zarray",
        "REF_qc/.zarray",
        "REF_encoded/.zarray",
        "REF_offsets/.zarray",
    ] {
        ensure!(
            inventory.contains_key(required),
            "incomplete artifact inventory: {required}"
        );
    }
    let mut shapes = std::collections::BTreeMap::new();
    let mut attributes = None;
    for (relative, entry) in &inventory {
        let bytes = read_bounded(store, &Path::from(entry.key.as_str()), entry.bytes).await?;
        ensure!(
            bytes.len() == entry.bytes && crate::checksum(&bytes) == entry.sha256,
            "artifact checksum/length mismatch"
        );
        let keep_it = keep(relative);
        if let Some(array) = relative.strip_suffix("/.zarray") {
            let metadata: serde_json::Value = serde_json::from_slice(&bytes)?;
            let shape = metadata["shape"][0]
                .as_u64()
                .context("invalid Zarr shape")? as usize;
            let chunk = metadata["chunks"][0]
                .as_u64()
                .context("invalid Zarr chunks")? as usize;
            let dtype = metadata["dtype"].as_str().context("invalid Zarr dtype")?;
            ensure!(
                shape > 0
                    && shape <= crate::RADIAL_LIMIT * crate::GATE_LIMIT * 2
                    && (1..=65536).contains(&chunk),
                "Zarr array cap"
            );
            ensure!(
                metadata["zarr_format"] == 2
                    && metadata["compressor"].is_null()
                    && metadata["order"] == "C",
                "unsupported Zarr encoding"
            );
            shapes.insert(array.to_owned(), shape);
            let width = match dtype {
                "<f4" => 4,
                "<i8" | "<u8" => 8,
                "|u1" => 1,
                _ => anyhow::bail!("unsupported Zarr dtype"),
            };
            for index in 0..shape.div_ceil(chunk) {
                let entry = inventory
                    .get(format!("{array}/{index}").as_str())
                    .context("missing numeric chunk")?;
                ensure!(entry.bytes == chunk * width, "invalid numeric chunk length");
            }
        }
        if *relative == ".zattrs" {
            let metadata: serde_json::Value = serde_json::from_slice(&bytes)?;
            ensure!(
                metadata["station"] == commit.station
                    && metadata["scan_end"] == serde_json::to_value(commit.scan_end)?,
                "artifact provenance mismatch"
            );
            let rays = metadata["rays"]
                .as_array()
                .context("missing ray geometry")?;
            ensure!(
                !rays.is_empty() && rays.len() <= crate::RADIAL_LIMIT,
                "artifact ray cap"
            );
            ensure!(
                rays.last()
                    .is_some_and(|r| r["radial_status"] == 2 || r["radial_status"] == 4),
                "incomplete artifact sweep"
            );
            attributes = Some(metadata);
        }
        if keep_it {
            kept.insert((*relative).to_owned(), bytes);
        }
    }
    let attributes = attributes.context("missing artifact attributes")?;
    let rays = attributes["rays"].as_array().context("missing rays")?;
    for name in ["time_ms", "azimuth_deg", "elevation_deg"] {
        ensure!(
            shapes.get(name) == Some(&rays.len()),
            "coordinate shape mismatch"
        );
    }
    for (i, name) in ["REF", "RHO", "ZDR"].iter().enumerate() {
        let mut gates = 0usize;
        let mut encoded = 0usize;
        for ray in rays {
            let m = &ray["moments"][i];
            if m.is_null() {
                continue;
            }
            let count = m["gates"].as_u64().context("missing gate count")? as usize;
            let bits = m["word_bits"].as_u64().context("missing word size")? as usize;
            ensure!(
                count <= crate::GATE_LIMIT && [8, 16].contains(&bits),
                "artifact moment cap"
            );
            gates += count;
            encoded += count * (bits / 8);
        }
        if gates > 0 {
            for (array, len) in [
                ((*name).to_owned(), gates),
                (format!("{name}_qc"), gates),
                (format!("{name}_encoded"), encoded),
                (format!("{name}_offsets"), rays.len() + 1),
            ] {
                ensure!(
                    shapes.get(&array) == Some(&len),
                    "moment shape/inventory mismatch"
                );
            }
        }
    }
    Ok((commit, kept))
}

pub(crate) async fn read_bounded(store: &dyn ObjectStore, path: &Path, limit: usize) -> Result<Vec<u8>> {
    let result = store.get(path).await?;
    ensure!(result.meta.size <= limit as u64, "artifact read size limit");
    let mut stream = result.into_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        ensure!(
            bytes
                .len()
                .checked_add(chunk.len())
                .is_some_and(|n| n <= limit),
            "artifact stream limit"
        );
        bytes.try_reserve_exact(chunk.len())?;
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    #[tokio::test]
    async fn immutable_commit_and_readback_fail_closed() -> Result<()> {
        let store = object_store::memory::InMemory::new();
        write_verified(&store, "level2/test/data", b"abc").await?;
        assert!(
            write_verified(&store, "level2/test/data", b"xyz")
                .await
                .is_err()
        );
        assert!(verify(&store, "level2/test/commit.json").await.is_err());
        let artifact = Artifact::build(&test_sweep(), vec![], "level2/test")?;
        artifact.publish(&store).await?;
        verify(&store, &artifact.commit_key).await?;
        let key = &artifact.objects[0].0;
        store
            .put(&Path::from(key.as_str()), b"bad".to_vec().into())
            .await?;
        assert!(verify(&store, &artifact.commit_key).await.is_err());
        Ok(())
    }
    #[tokio::test]
    async fn interrupted_publication_never_writes_commit() -> Result<()> {
        let mut sweep = test_sweep();
        let artifact = Artifact::build(&sweep, vec![], "level2/test")?;
        let store = object_store::memory::InMemory::new();
        let (key, _) = &artifact.objects[2];
        store
            .put(&Path::from(key.as_str()), b"interrupt".to_vec().into())
            .await?;
        assert!(artifact.publish(&store).await.is_err());
        assert!(
            store
                .get(&Path::from(artifact.commit_key.as_str()))
                .await
                .is_err()
        );
        sweep.rays.pop();
        sweep.rays.last_mut().unwrap().radial_status = 1;
        assert!(Artifact::build(&sweep, vec![], "level2/test").is_err());
        Ok(())
    }
    pub(crate) fn test_sweep() -> Sweep {
        use crate::decode::{Moment, Ray};
        let now = chrono::Utc::now();
        let rays = (1..=360)
            .map(|number| Ray {
                number,
                time: now,
                azimuth_deg: (number - 1) as f32,
                elevation_deg: 0.5,
                azimuth_spacing_deg: 1.0,
                radial_status: if number == 1 {
                    0
                } else if number == 360 {
                    2
                } else {
                    1
                },
                blanking: 0,
                moments: [
                    Some(Moment {
                        first_gate_m: 0,
                        spacing_m: 250,
                        word_bits: 8,
                        scale: 2.0,
                        offset: 66.0,
                        control_flags: 0,
                        snr_threshold_raw: 0,
                        tover_raw: 0,
                        encoded: vec![100],
                        values: vec![17.0],
                        qc: vec![0],
                    }),
                    None,
                    None,
                ],
            })
            .collect();
        Sweep {
            station: "KGRR".into(),
            volume_start: now,
            volume_number: "001".into(),
            ordinal: 1,
            elevation_number: 1,
            vcp: Some(35),
            site: Some(crate::decode::VolumeSite {
                latitude_deg: 42.893_89,
                longitude_deg: -85.544_89,
                site_height_m_raw: 0,
                tower_height_m_raw: 0,
            }),
            scan_start: now,
            scan_end: now,
            rays,
            missing_sectors_deg: vec![],
        }
    }
}
