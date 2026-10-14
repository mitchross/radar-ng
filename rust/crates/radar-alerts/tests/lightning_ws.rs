//! Lightning consumer against a local websocket server (spec-ingest-aux section 6.4 to 6.10).

use std::collections::BTreeSet;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use futures_util::{SinkExt, StreamExt};
use radar_alerts::lightning::lightning_consume_stream;
use radar_alerts::lightning::{
    LightningConfig, LightningError, SUBSCRIBE_FRAME, Strike, decode_frame, decode_payload,
    load_existing_strikes, render_geojson,
};
use serde_json::{Value, json};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;
use tokio_util::sync::CancellationToken;

fn now_epoch() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
}

/// Plain LZW encoder matching the Blitzortung variant: literals below 256, dictionary codes from 256 in insertion
/// order. Used to build compressed text frames for the decoder.
fn lzw_encode(text: &str) -> String {
    fn known(s: &str, dict: &HashMap<String, u32>) -> bool {
        s.chars().count() == 1 || dict.contains_key(s)
    }
    fn code_char(s: &str, dict: &HashMap<String, u32>) -> char {
        if s.chars().count() == 1 {
            s.chars().next().unwrap()
        } else {
            char::from_u32(dict[s]).unwrap()
        }
    }
    let mut dict: HashMap<String, u32> = HashMap::new();
    let mut next_code = 256u32;
    let mut out = String::new();
    let mut chars = text.chars();
    let Some(first) = chars.next() else {
        return out;
    };
    let mut w = first.to_string();
    for c in chars {
        let wc = format!("{w}{c}");
        if known(&wc, &dict) {
            w = wc;
        } else {
            out.push(code_char(&w, &dict));
            dict.insert(wc, next_code);
            next_code += 1;
            w = c.to_string();
        }
    }
    out.push(code_char(&w, &dict));
    out
}

/// A local websocket endpoint. The first connection receives `script`, then the server closes it. Later connections
/// receive nothing and stay open until the client goes away.
struct Server {
    url: String,
    connections: Arc<AtomicUsize>,
    subscribes: Arc<Mutex<Vec<String>>>,
}

async fn spawn_server(script: Vec<Message>) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let connections = Arc::new(AtomicUsize::new(0));
    let subscribes = Arc::new(Mutex::new(Vec::new()));
    let conns = Arc::clone(&connections);
    let subs = Arc::clone(&subscribes);
    tokio::spawn(async move {
        let mut script = Some(script);
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let n = conns.fetch_add(1, Ordering::SeqCst);
            let frames = if n == 0 {
                script.take().unwrap_or_default()
            } else {
                Vec::new()
            };
            let subs = Arc::clone(&subs);
            tokio::spawn(async move {
                let Ok(mut ws) = tokio_tungstenite::accept_async(stream).await else {
                    return;
                };
                if let Some(Ok(Message::Text(text))) = ws.next().await {
                    subs.lock().unwrap().push(text.to_string());
                }
                for frame in frames {
                    if ws.send(frame).await.is_err() {
                        return;
                    }
                }
                if n == 0 {
                    let _ = ws.close(None).await;
                }
                while ws.next().await.is_some() {}
            });
        }
    });
    Server {
        url: format!("ws://{addr}/"),
        connections,
        subscribes,
    }
}

fn config(dir: &std::path::Path, url: &str) -> LightningConfig {
    let mut cfg = LightningConfig::new(dir);
    cfg.primary = vec![url.to_owned()];
    cfg.legacy = Vec::new();
    cfg.heartbeat_every = Duration::from_millis(100);
    cfg
}

fn strike_json(lat: f64, lon: f64, time_ns: i64) -> String {
    json!({"lat": lat, "lon": lon, "time": time_ns, "pol": 1, "mds": 5}).to_string()
}

#[test]
fn lzw_helper_round_trips_through_the_decoder() {
    let text = r#"{"lat":37.0,"lon":-95.0,"time":1700000000000000000}"#;
    let encoded = lzw_encode(text);
    assert_ne!(encoded, text);
    assert_eq!(decode_payload(&encoded), text);
    // The encoded form is not JSON, so the decoder is what recovers the strike.
    assert!(serde_json::from_str::<Value>(&encoded).is_err());
    assert!(decode_frame(&encoded).is_some());
}

#[tokio::test]
async fn persists_strikes_skips_bad_frames_and_reconnects_after_close() {
    let now_ns = (now_epoch() * 1e9) as i64;
    let lzw_text = lzw_encode(&strike_json(37.0, -95.0, now_ns));
    let script = vec![
        Message::Text(strike_json(36.1, -97.2, now_ns).into()),
        Message::Text(
            json!({"lat": 60.0, "lon": -97.0, "time": now_ns})
                .to_string()
                .into(),
        ),
        Message::Text("[1, 2, 3]".into()),
        Message::Text(
            json!({"lat": null, "lon": -97.0, "time": 5})
                .to_string()
                .into(),
        ),
        Message::Binary(strike_json(35.0, -96.0, now_ns).into_bytes().into()),
        Message::Text("null".into()),
        Message::Text(lzw_text.into()),
    ];
    let server = spawn_server(script).await;
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(dir.path(), &server.url);
    let out_path = cfg.out_path();

    let beats: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let beats_cb = Arc::clone(&beats);
    let result = lightning_consume_stream(
        cfg,
        Duration::from_millis(2500),
        CancellationToken::new(),
        move |details: Value| beats_cb.lock().unwrap().push(details),
    )
    .await
    .unwrap();

    // Seven data messages arrived. The `null` frame is not counted as parsed. The array, the bad-field object,
    // the out-of-box strike and the three in-box strikes are parsed.
    assert_eq!(result.msgs, 7);
    assert_eq!(result.parsed, 6);
    assert_eq!(result.in_bbox, 3);
    assert_eq!(result.final_buffer, 3);
    assert_eq!(result.seeded, 0);
    // The second connection delivered nothing before the deadline, which counts as one connect failure.
    assert_eq!(result.connect_failures, 1);

    // Reconnect after the close: a second connection was made, and both subscribed with the exact frame.
    assert_eq!(server.connections.load(Ordering::SeqCst), 2);
    let subscribes = server.subscribes.lock().unwrap().clone();
    assert!(!subscribes.is_empty());
    assert!(subscribes.iter().all(|s| s == SUBSCRIBE_FRAME));

    // The published file holds exactly the three in-box strikes (text, binary and LZW-compressed).
    let persisted = load_existing_strikes(&out_path, 15, now_epoch(), 100);
    let mut coords: Vec<(i64, i64)> = persisted
        .iter()
        .map(|s| ((s.lat * 1000.0) as i64, (s.lon * 1000.0) as i64))
        .collect();
    coords.sort_unstable();
    assert_eq!(
        coords,
        vec![(35000, -96000), (36100, -97200), (37000, -95000)]
    );

    // Heartbeats carry the counters every 100 ms, so the run produced several and saw all seven messages.
    let beats = beats.lock().unwrap().clone();
    assert!(
        beats.len() >= 3,
        "expected several heartbeats, got {}",
        beats.len()
    );
    let max_msgs = beats
        .iter()
        .filter_map(|b| b["msgs"].as_i64())
        .max()
        .unwrap_or(0);
    assert_eq!(max_msgs, 7);
    let last = beats.last().unwrap();
    for key in ["msgs", "parsed", "in_bbox", "buffer", "seeded"] {
        assert!(
            last.get(key).is_some(),
            "heartbeat is missing {key}: {last}"
        );
    }
}

#[tokio::test]
async fn cancellation_returns_cancelled_promptly() {
    let server = spawn_server(Vec::new()).await;
    let dir = tempfile::tempdir().unwrap();
    let cancel = CancellationToken::new();
    let trigger = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(300)).await;
        trigger.cancel();
    });
    let started = Instant::now();
    let err = lightning_consume_stream(
        config(dir.path(), &server.url),
        Duration::from_secs(60),
        cancel,
        |_: Value| {},
    )
    .await
    .unwrap_err();
    assert!(matches!(err, LightningError::Cancelled), "got {err:?}");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "cancel took {:?}",
        started.elapsed()
    );
}

#[tokio::test]
async fn seeds_from_previous_file_inside_the_retention_window() {
    let now = now_epoch();
    let previous = vec![
        Strike {
            t: now - 60.0,
            lat: 36.0,
            lon: -97.0,
            pol: 0,
            mds: 0,
        },
        Strike {
            t: now - 3600.0,
            lat: 36.5,
            lon: -97.5,
            pol: 0,
            mds: 0,
        },
    ];
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(dir.path(), "ws://127.0.0.1:1/");
    std::fs::write(cfg.out_path(), render_geojson(&previous, now, 15)).unwrap();
    let server = spawn_server(Vec::new()).await;

    let result = lightning_consume_stream(
        config(dir.path(), &server.url),
        Duration::from_millis(400),
        CancellationToken::new(),
        |_: Value| {},
    )
    .await
    .unwrap();

    assert_eq!(
        result.seeded, 1,
        "the hour-old strike is outside the 15 minute window"
    );
    assert_eq!(result.final_buffer, 1);
    assert_eq!(result.msgs, 0);
    let persisted = load_existing_strikes(&cfg.out_path(), 15, now_epoch(), 100);
    assert_eq!(persisted.len(), 1);
    assert!((persisted[0].t - (now - 60.0)).abs() < 1e-3);
}

#[tokio::test]
async fn refused_endpoint_is_retried_and_counted_without_panicking() {
    // Bind and drop a listener to get a local port that refuses connections.
    let port = {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        listener.local_addr().unwrap().port()
    };
    let dir = tempfile::tempdir().unwrap();
    let cfg = config(dir.path(), &format!("ws://127.0.0.1:{port}/"));
    let result = lightning_consume_stream(
        cfg,
        Duration::from_millis(500),
        CancellationToken::new(),
        |_: Value| {},
    )
    .await
    .unwrap();
    assert_eq!(result.msgs, 0);
    assert!(result.connect_failures >= 1);
    let names: BTreeSet<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(names.contains("lightning.json"));
}
