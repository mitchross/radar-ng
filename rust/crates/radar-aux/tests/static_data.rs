//! The temperature normals store: the HDF5 reader against a cut of the real NCEI file, checked against
//! published station normals, and the build end to end on a tiny synthetic file over HTTP ranges.

use std::collections::BTreeMap;
use std::io;
use std::sync::Arc;

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::IntoResponse;
use chrono::NaiveDate;
use radar_aux::static_data::hdf5::{self, ReadAt};
use radar_aux::static_data::normals::{self, DAYS, Normals, Var};
use radar_aux::static_data::{StaticDataConfig, ensure_static_data};

// ---------------------------------------------------------------------------
// A sparse cut of the real file: only the byte ranges the reader touches.
// ---------------------------------------------------------------------------

/// Byte ranges of a file, `offset -> bytes`.
#[derive(Default)]
struct Sparse(BTreeMap<u64, Vec<u8>>);

impl Sparse {
    fn parse(b: &[u8]) -> Sparse {
        let mut s = Sparse::default();
        let mut i = 0;
        while i < b.len() {
            let off = u64::from_le_bytes(b[i..i + 8].try_into().unwrap());
            let len = u32::from_le_bytes(b[i + 8..i + 12].try_into().unwrap()) as usize;
            s.0.insert(off, b[i + 12..i + 12 + len].to_vec());
            i += 12 + len;
        }
        s
    }

    fn encode(&self) -> Vec<u8> {
        let mut out = Vec::new();
        for (off, b) in &self.0 {
            out.extend_from_slice(&off.to_le_bytes());
            out.extend_from_slice(&(b.len() as u32).to_le_bytes());
            out.extend_from_slice(b);
        }
        out
    }
}

impl ReadAt for Sparse {
    fn read_at(&mut self, offset: u64, len: usize) -> io::Result<Vec<u8>> {
        let (start, b) = self
            .0
            .range(..=offset)
            .next_back()
            .ok_or_else(|| io::Error::other(format!("{offset} not in the cut")))?;
        let i = (offset - start) as usize;
        b.get(i..i + len)
            .map(<[u8]>::to_vec)
            .ok_or_else(|| io::Error::other(format!("{offset}+{len} not in the cut")))
    }
}

/// Records every read of an inner reader, to cut a fixture from the real file.
struct Recording<R: ReadAt> {
    inner: R,
    cut: Sparse,
}

impl<R: ReadAt> ReadAt for Recording<R> {
    fn read_at(&mut self, offset: u64, len: usize) -> io::Result<Vec<u8>> {
        let b = self.inner.read_at(offset, len)?;
        self.cut.0.insert(offset, b.clone());
        Ok(b)
    }
}

/// Published 1991-2020 station normals (NCEI `normals-daily/1991-2020/access/<id>.csv`), degrees F:
/// `(name, lat, lon, [(month, day, tmax, tmin)])`.
type Station = (&'static str, f64, f64, [(u32, u32, f64, f64); 5]);
const STATIONS: [Station; 3] = [
    (
        "USW00094847 Detroit Metro",
        42.2314,
        -83.3308,
        [
            (1, 1, 33.6, 21.4),
            (2, 28, 39.3, 23.9),
            (2, 29, 39.5, 24.1),
            (3, 1, 39.7, 24.2),
            (7, 15, 84.0, 64.6),
        ],
    ),
    (
        "USW00094846 Chicago O'Hare",
        41.9950,
        -87.9336,
        [
            (1, 1, 32.9, 20.8),
            (2, 28, 40.5, 26.1),
            (2, 29, 40.7, 26.2),
            (3, 1, 40.9, 26.4),
            (7, 15, 84.8, 66.7),
        ],
    ),
    (
        "USW00023183 Phoenix Sky Harbor",
        33.4278,
        -112.0039,
        [
            (1, 1, 65.7, 44.7),
            (2, 28, 73.4, 51.3),
            (2, 29, 73.5, 51.4),
            (3, 1, 73.7, 51.5),
            (7, 15, 106.7, 84.9),
        ],
    ),
];

fn fixture_path(var: Var) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("testdata/static_data")
        .join(format!("{}-cut.bin", var.name()))
}

/// Each station's day values, degrees C, read the way the converter reads them.
fn station_values(r: &mut dyn ReadAt, var: Var) -> Vec<Vec<f32>> {
    let grid = normals::read_grid(r).unwrap();
    let chunks = normals::day_chunks(r, var, &grid).unwrap();
    STATIONS
        .iter()
        .map(|(_, lat, lon, days)| {
            let (row, col) = grid.cell(*lat, *lon).unwrap();
            days.iter()
                .map(|&(m, d, _, _)| {
                    let day = normals::day_index(NaiveDate::from_ymd_opt(2028, m, d).unwrap());
                    let at = chunks[day].address + ((row * grid.cols + col) * 4) as u64;
                    f32::from_le_bytes(r.read_at(at, 4).unwrap().try_into().unwrap())
                })
                .collect()
        })
        .collect()
}

/// Cut the fixtures from the real files: `NORMALS_FIXTURE_SRC=<dir with tmax.nc, tmin.nc>`.
#[test]
fn write_fixture_from_real_files() {
    let Ok(src) = std::env::var("NORMALS_FIXTURE_SRC") else {
        return;
    };
    for var in Var::ALL {
        let file = std::fs::File::open(format!("{src}/{}.nc", var.name())).unwrap();
        let mut rec = Recording {
            inner: file,
            cut: Sparse::default(),
        };
        station_values(&mut rec, var);
        let path = fixture_path(var);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, rec.cut.encode()).unwrap();
    }
}

#[test]
fn real_file_layout_and_known_points_match_station_normals() {
    for var in Var::ALL {
        let mut cut = Sparse::parse(&std::fs::read(fixture_path(var)).unwrap());
        let grid = normals::read_grid(&mut cut).unwrap();
        assert_eq!((grid.rows, grid.cols), (596, 1385));
        assert!((grid.lat0 - 24.5625).abs() < 1e-4 && (grid.lon0 + 124.6875).abs() < 1e-4);
        assert!((grid.step - 1.0 / 24.0).abs() < 1e-12);
        assert_eq!(grid.file_bytes(), 604_236_720);
        let values = station_values(&mut cut, var);
        for ((name, _, _, days), got) in STATIONS.iter().zip(values) {
            for (&(m, d, tmax, tmin), c) in days.iter().zip(got) {
                let want = if var == Var::Tmax { tmax } else { tmin };
                let f = f64::from(c) * 1.8 + 32.0;
                // A 4 km cell is not the airport thermometer. Highs agree within 1 F; airport lows run
                // warm (Phoenix by about 5 F, the urban heat island), so lows get a wider band.
                let tol = if var == Var::Tmax { 1.0 } else { 5.5 };
                assert!(
                    (f - want).abs() <= tol,
                    "{name} {m}/{d} {}: grid {f:.1} F, station {want} F",
                    var.name()
                );
            }
        }
    }
}

#[test]
fn feb_29_sits_between_feb_28_and_mar_1() {
    let mut cut = Sparse::parse(&std::fs::read(fixture_path(Var::Tmax)).unwrap());
    for v in station_values(&mut cut, Var::Tmax) {
        // Days are Jan 1, Feb 28, Feb 29, Mar 1, Jul 15. Late February warms day by day.
        assert!(v[1] < v[2] && v[2] < v[3], "{v:?}");
    }
}

// ---------------------------------------------------------------------------
// A tiny synthetic NetCDF-4/HDF5 file with the same structure.
// ---------------------------------------------------------------------------

const ROWS: usize = 3;
const COLS: usize = 4;
const LAT0: f32 = 40.0;
const LON0: f32 = -90.0;

fn synthetic(var: Var, day: usize, r: usize, c: usize) -> f32 {
    if (r, c) == (0, 0) {
        return f32::NAN; // a water cell
    }
    let high = 10.0 + day as f32 * 0.05 + r as f32 + c as f32 * 0.1;
    if var == Var::Tmax { high } else { high - 10.0 }
}

fn msg(t: u8, body: &[u8]) -> Vec<u8> {
    let mut m = vec![t];
    m.extend_from_slice(&(body.len() as u16).to_le_bytes());
    m.push(0);
    m.extend_from_slice(body);
    m
}

fn ohdr(msgs: &[Vec<u8>]) -> Vec<u8> {
    let body: Vec<u8> = msgs.concat();
    let mut h = b"OHDR".to_vec();
    h.extend_from_slice(&[2, 0x02]);
    h.extend_from_slice(&(body.len() as u32).to_le_bytes());
    h.extend_from_slice(&body);
    h.extend_from_slice(&[0; 4]); // checksum (not verified by the reader)
    h
}

fn float_type() -> Vec<u8> {
    let mut b = vec![0x11, 0x20, 0x1f, 0x00];
    b.extend_from_slice(&4u32.to_le_bytes());
    b.extend_from_slice(&[0, 0, 32, 0, 23, 8, 0, 23, 127, 0, 0, 0]);
    msg(0x03, &b)
}

fn dataspace(dims: &[u64]) -> Vec<u8> {
    let mut b = vec![2, dims.len() as u8, 0, 1];
    for d in dims {
        b.extend_from_slice(&d.to_le_bytes());
    }
    msg(0x01, &b)
}

fn contiguous(addr: u64, size: u64) -> Vec<u8> {
    let mut b = vec![3, 1];
    b.extend_from_slice(&addr.to_le_bytes());
    b.extend_from_slice(&size.to_le_bytes());
    msg(0x08, &b)
}

fn chunked(btree: u64) -> Vec<u8> {
    let mut b = vec![3, 2, 4];
    b.extend_from_slice(&btree.to_le_bytes());
    for d in [1u32, ROWS as u32, COLS as u32, 4] {
        b.extend_from_slice(&d.to_le_bytes());
    }
    msg(0x08, &b)
}

fn link(name: &str, addr: u64) -> Vec<u8> {
    let mut b = vec![1, 0, name.len() as u8];
    b.extend_from_slice(name.as_bytes());
    b.extend_from_slice(&addr.to_le_bytes());
    msg(0x06, &b)
}

/// The file bytes: superblock, root group, `lat`/`lon`/variable headers, B-tree, axes, day chunks.
/// Headers are laid out at fixed slots so their addresses are known up front.
fn synthetic_file(var: Var) -> Vec<u8> {
    let (root, lat_h, lon_h, var_h, btree, axes) =
        (48u64, 512u64, 768u64, 1024u64, 1536u64, 32768u64);
    let lat_addr = axes;
    let lon_addr = axes + (ROWS * 4) as u64;
    let data = 65536u64;
    let plane = (ROWS * COLS * 4) as u64;
    let mut f = vec![0u8; (data + plane * DAYS as u64) as usize];
    let put = |f: &mut Vec<u8>, at: u64, b: &[u8]| {
        f[at as usize..at as usize + b.len()].copy_from_slice(b)
    };

    let mut sb = b"\x89HDF\r\n\x1a\n".to_vec();
    sb.extend_from_slice(&[2, 8, 8, 0]);
    sb.extend_from_slice(&0u64.to_le_bytes());
    sb.extend_from_slice(&u64::MAX.to_le_bytes());
    sb.extend_from_slice(&(f.len() as u64).to_le_bytes());
    sb.extend_from_slice(&root.to_le_bytes());
    sb.extend_from_slice(&[0; 4]);
    put(&mut f, 0, &sb);
    put(
        &mut f,
        root,
        &ohdr(&[
            link("lat", lat_h),
            link("lon", lon_h),
            link(var.nc_variable(), var_h),
        ]),
    );
    put(
        &mut f,
        lat_h,
        &ohdr(&[
            dataspace(&[ROWS as u64]),
            float_type(),
            contiguous(lat_addr, (ROWS * 4) as u64),
        ]),
    );
    put(
        &mut f,
        lon_h,
        &ohdr(&[
            dataspace(&[COLS as u64]),
            float_type(),
            contiguous(lon_addr, (COLS * 4) as u64),
        ]),
    );
    put(
        &mut f,
        var_h,
        &ohdr(&[
            dataspace(&[DAYS as u64, ROWS as u64, COLS as u64]),
            float_type(),
            chunked(btree),
        ]),
    );
    let step = 1.0f32 / 24.0;
    for i in 0..ROWS {
        put(
            &mut f,
            lat_addr + 4 * i as u64,
            &(LAT0 + step * i as f32).to_le_bytes(),
        );
    }
    for i in 0..COLS {
        put(
            &mut f,
            lon_addr + 4 * i as u64,
            &(LON0 + step * i as f32).to_le_bytes(),
        );
    }

    // One leaf node indexing every day, children in reverse to check the reader sorts.
    let mut node = b"TREE".to_vec();
    node.extend_from_slice(&[1, 0]);
    node.extend_from_slice(&(DAYS as u16).to_le_bytes());
    node.extend_from_slice(&u64::MAX.to_le_bytes());
    node.extend_from_slice(&u64::MAX.to_le_bytes());
    let key = |day: u64| {
        let mut k = (plane as u32).to_le_bytes().to_vec();
        k.extend_from_slice(&0u32.to_le_bytes());
        for o in [day, 0, 0, 0] {
            k.extend_from_slice(&o.to_le_bytes());
        }
        k
    };
    for day in 0..DAYS as u64 {
        let slot = DAYS as u64 - 1 - day;
        node.extend_from_slice(&key(day));
        node.extend_from_slice(&(data + slot * plane).to_le_bytes());
        let mut chunk = Vec::with_capacity(plane as usize);
        for r in 0..ROWS {
            for c in 0..COLS {
                chunk.extend_from_slice(&synthetic(var, day as usize, r, c).to_le_bytes());
            }
        }
        put(&mut f, data + slot * plane, &chunk);
    }
    node.extend_from_slice(&key(DAYS as u64));
    assert!(btree + (node.len() as u64) < axes);
    put(&mut f, btree, &node);
    f
}

/// Served files, by URL path.
type Files = Arc<Vec<(String, Vec<u8>)>>;

/// A local server answering `Range: bytes=a-b` with 206, as NCEI does.
async fn range_server(files: Vec<(String, Vec<u8>)>) -> String {
    async fn handle(
        State(files): State<Files>,
        uri: axum::http::Uri,
        headers: HeaderMap,
    ) -> axum::response::Response {
        let Some((_, body)) = files.iter().find(|(p, _)| *p == uri.path()) else {
            return StatusCode::NOT_FOUND.into_response();
        };
        let Some(range) = headers
            .get(header::RANGE)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("bytes="))
        else {
            return (StatusCode::OK, body.clone()).into_response();
        };
        let (a, b) = range.split_once('-').unwrap();
        let a: usize = a.parse().unwrap();
        let b: usize = b.parse::<usize>().unwrap().min(body.len() - 1);
        (StatusCode::PARTIAL_CONTENT, body[a..=b].to_vec()).into_response()
    }
    let app = Router::new().fallback(handle).with_state(Arc::new(files));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    format!("http://{addr}/normals")
}

#[test]
fn synthetic_file_parses_like_the_real_one() {
    let mut f = io::Cursor::new(synthetic_file(Var::Tmax));
    struct C<'a>(&'a mut io::Cursor<Vec<u8>>);
    impl ReadAt for C<'_> {
        fn read_at(&mut self, offset: u64, len: usize) -> io::Result<Vec<u8>> {
            let b = self.0.get_ref();
            b.get(offset as usize..offset as usize + len)
                .map(<[u8]>::to_vec)
                .ok_or_else(|| io::Error::other("eof"))
        }
    }
    let mut r = C(&mut f);
    let links = hdf5::root_links(&mut r).unwrap();
    assert_eq!(links.len(), 3);
    let grid = normals::read_grid(&mut r).unwrap();
    assert_eq!((grid.rows, grid.cols), (ROWS, COLS));
    let chunks = normals::day_chunks(&mut r, Var::Tmax, &grid).unwrap();
    assert_eq!(chunks.len(), DAYS);
    assert!(chunks[0].address > chunks[1].address);
}

#[tokio::test(flavor = "multi_thread")]
async fn ensure_builds_once_then_reports_present() {
    let base = range_server(
        Var::ALL
            .iter()
            .map(|&v| {
                let url = v.source_url("http://x/normals");
                (
                    url.trim_start_matches("http://x").to_string(),
                    synthetic_file(v),
                )
            })
            .collect(),
    )
    .await;
    let state = tempfile::tempdir().unwrap();
    let config = StaticDataConfig {
        state_dir: state.path().to_path_buf(),
        normals_base_url: base,
    };
    let client = reqwest::Client::new();
    let beats = Arc::new(std::sync::Mutex::new(0usize));
    let counter = beats.clone();
    let beat: radar_aux::Beat = Arc::new(move |_| *counter.lock().unwrap() += 1);

    assert!(Normals::open(state.path()).unwrap().is_none());
    let built = ensure_static_data(&config, &client, "normals", beat.clone())
        .await
        .unwrap();
    assert_eq!(built.outcome, "built");
    let expected = 2 * (DAYS * ROWS * COLS * 2) as i64;
    assert_eq!(built.bytes, Some(expected));
    assert!(*beats.lock().unwrap() >= 2 * 36);

    let n = Normals::open(state.path())
        .unwrap()
        .expect("marker written");
    let lat = f64::from(LAT0) + 1.0 / 24.0;
    let lon = f64::from(LON0) + 2.0 / 24.0;
    let day = |y, m, d| NaiveDate::from_ymd_opt(y, m, d).unwrap();
    // Row 1, col 2: high 10 + day/20 + 1 + 0.2.
    let leap = n.point(lat, lon, day(2028, 2, 29)).unwrap().unwrap();
    assert!(
        (leap.high_c - (11.2 + 59.0 * 0.05)).abs() < 0.051,
        "{leap:?}"
    );
    assert!((leap.high_c - leap.low_c - 10.0).abs() < 0.051);
    let mar1 = n.point(lat, lon, day(2026, 3, 1)).unwrap().unwrap();
    assert!(
        (mar1.high_c - (11.2 + 60.0 * 0.05)).abs() < 0.051,
        "{mar1:?}"
    );
    // The water cell borrows its nearest land neighbour; far away is nothing.
    let coast = n
        .point(f64::from(LAT0), f64::from(LON0), day(2026, 1, 1))
        .unwrap()
        .unwrap();
    assert!((coast.high_c - 11.0).abs() < 0.051, "{coast:?}");
    assert!(n.point(10.0, -150.0, day(2026, 1, 1)).unwrap().is_none());

    // The marker makes the next run free: a dead server is never asked.
    let offline = StaticDataConfig {
        normals_base_url: "http://127.0.0.1:9/normals".into(),
        ..config
    };
    let again = ensure_static_data(&offline, &client, "normals", beat)
        .await
        .unwrap();
    assert_eq!(again.outcome, "present");
    assert_eq!(again.bytes, Some(expected));
}

#[tokio::test]
async fn unknown_product_is_an_error() {
    let state = tempfile::tempdir().unwrap();
    let config = StaticDataConfig {
        state_dir: state.path().to_path_buf(),
        normals_base_url: "http://127.0.0.1:9".into(),
    };
    let err = ensure_static_data(
        &config,
        &reqwest::Client::new(),
        "dem",
        radar_aux::no_beat(),
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("dem"));
}

/// Full build from the real files, for the PR's measured bytes: `NORMALS_FIXTURE_SRC=<dir with
/// tmax.nc, tmin.nc> NORMALS_BUILD_STATE=<state dir>`. Skipped otherwise.
#[test]
fn full_build_from_real_files() {
    let (Ok(src), Ok(state)) = (
        std::env::var("NORMALS_FIXTURE_SRC"),
        std::env::var("NORMALS_BUILD_STATE"),
    ) else {
        return;
    };
    let state = std::path::PathBuf::from(state);
    let beat: radar_aux::beat::Beat = Arc::new(|_| {});
    let mut open = |var: Var| -> io::Result<Box<dyn ReadAt>> {
        Ok(Box::new(std::fs::File::open(format!(
            "{src}/{}.nc",
            var.name()
        ))?))
    };
    let meta = radar_aux::static_data::build_normals(&state, &[], &mut open, &beat).unwrap();
    assert_eq!(meta.bytes, 2 * 604_236_720);
    let n = Normals::open(&state).unwrap().expect("marker written");
    for (name, lat, lon, days) in STATIONS {
        for (m, d, tmax, tmin) in days {
            let date = NaiveDate::from_ymd_opt(2024, m, d).unwrap();
            let got = n.point(lat, lon, date).unwrap().expect("land cell");
            let (hi, lo) = (got.high_c * 1.8 + 32.0, got.low_c * 1.8 + 32.0);
            assert!(
                (hi - tmax).abs() <= 1.0,
                "{name} {m}/{d} high {hi:.1} vs {tmax}"
            );
            assert!(
                (lo - tmin).abs() <= 5.5,
                "{name} {m}/{d} low {lo:.1} vs {tmin}"
            );
        }
    }
}
