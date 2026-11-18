//! Nice Day and Bonfire score for one forecast hour, the Rust twin of `web/src/shared/niceDay.ts`.
//!
//! Our own index, not an official one. Both read `web/src/shared/testdata/niceDay.vectors.json` in
//! their tests, so the map layer and the card give the same word. Units: °F, mph, %, inches.

/// Which table to score with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Daytime comfort.
    NiceDay,
    /// Evening fire: light wind first, then dry, then cool but not cold.
    Bonfire,
}

/// One forecast hour; `None` (or a non-finite value) is unknown.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Hour {
    /// Air temperature, °F.
    pub temp_f: Option<f64>,
    /// Feels-like temperature, °F. Wins over `temp_f`.
    pub feels_f: Option<f64>,
    /// Sustained wind, mph.
    pub wind_mph: Option<f64>,
    /// Chance of rain, %.
    pub rain_chance: Option<f64>,
    /// Rain in the hour, inches.
    pub rain_in: Option<f64>,
    /// Relative humidity, %.
    pub humidity: Option<f64>,
}

/// `[input, points off]` pairs, linear between them, flat past the ends.
type Ramp = &'static [(f64, f64)];

struct Table {
    temp: Option<Ramp>,
    wind: Option<Ramp>,
    rain_chance: Option<Ramp>,
    rain_amount: Option<Ramp>,
    humidity: Option<Ramp>,
    humid_above_f: f64,
    max_wind_mph: f64,
    needs_wind: bool,
}

const NICE_DAY: Table = Table {
    temp: Some(&[
        (0.0, 100.0),
        (25.0, 65.0),
        (40.0, 40.0),
        (50.0, 20.0),
        (62.0, 0.0),
        (78.0, 0.0),
        (85.0, 25.0),
        (92.0, 55.0),
        (100.0, 100.0),
    ]),
    wind: Some(&[
        (10.0, 0.0),
        (15.0, 10.0),
        (20.0, 25.0),
        (30.0, 50.0),
        (40.0, 80.0),
    ]),
    rain_chance: Some(&[(20.0, 0.0), (50.0, 15.0), (80.0, 30.0), (100.0, 40.0)]),
    rain_amount: Some(&[
        (0.0, 0.0),
        (0.01, 20.0),
        (0.05, 40.0),
        (0.1, 55.0),
        (0.3, 90.0),
    ]),
    humidity: Some(&[(60.0, 0.0), (75.0, 10.0), (90.0, 25.0), (100.0, 35.0)]),
    humid_above_f: 70.0,
    max_wind_mph: f64::INFINITY,
    needs_wind: false,
};

const BONFIRE: Table = Table {
    temp: Some(&[
        (0.0, 100.0),
        (20.0, 55.0),
        (32.0, 25.0),
        (42.0, 0.0),
        (65.0, 0.0),
        (75.0, 20.0),
        (85.0, 45.0),
        (95.0, 70.0),
    ]),
    wind: Some(&[(5.0, 0.0), (10.0, 15.0), (15.0, 35.0), (20.0, 60.0)]),
    rain_chance: Some(&[(10.0, 0.0), (40.0, 25.0), (70.0, 50.0), (100.0, 70.0)]),
    rain_amount: Some(&[(0.0, 0.0), (0.01, 40.0), (0.05, 80.0), (0.1, 100.0)]),
    humidity: None,
    humid_above_f: f64::INFINITY,
    max_wind_mph: 20.0,
    needs_wind: true,
};

/// The five words, best first, with the lowest score of each.
pub const WORDS: [(f64, &str); 5] = [
    (80.0, "Great"),
    (60.0, "Good"),
    (40.0, "Fair"),
    (20.0, "Poor"),
    (0.0, "Do Not"),
];

/// The word for a score.
pub fn word_for(score: f64) -> &'static str {
    WORDS
        .iter()
        .find(|(min, _)| score >= *min)
        .map_or("Do Not", |(_, w)| w)
}

fn ramp(points: Ramp, x: f64) -> f64 {
    if x <= points[0].0 {
        return points[0].1;
    }
    for pair in points.windows(2) {
        let ((x0, y0), (x1, y1)) = (pair[0], pair[1]);
        if x <= x1 {
            return y0 + (y1 - y0) * (x - x0) / (x1 - x0);
        }
    }
    points[points.len() - 1].1
}

fn known(v: Option<f64>) -> Option<f64> {
    v.filter(|x| x.is_finite())
}

/// Score one hour, 0 to 100: 100 minus each factor's points off, rounded half up like
/// `Math.round`, floored at 0. `None` when temperature (or wind, for Bonfire) is unknown.
pub fn score_hour(hour: &Hour, mode: Mode) -> Option<f64> {
    let t = match mode {
        Mode::NiceDay => &NICE_DAY,
        Mode::Bonfire => &BONFIRE,
    };
    let temp = known(hour.feels_f).or(known(hour.temp_f))?;
    let wind = known(hour.wind_mph);
    if t.needs_wind && wind.is_none() {
        return None;
    }
    if wind.is_some_and(|w| w > t.max_wind_mph) {
        return Some(0.0);
    }
    let humidity = (temp >= t.humid_above_f)
        .then(|| known(hour.humidity))
        .flatten();
    let off: f64 = [
        (t.temp, Some(temp)),
        (t.wind, wind),
        (t.rain_chance, known(hour.rain_chance)),
        (t.rain_amount, known(hour.rain_in)),
        (t.humidity, humidity),
    ]
    .into_iter()
    .filter_map(|(r, x)| Some(ramp(r?, x?)))
    .sum();
    Some((100.0 - off + 0.5).floor().max(0.0))
}

/// The `nice-day` map value of one HRRR cell: the Nice Day score, or NaN when it cannot be scored.
/// HRRR has no rain chance, so the map scores rain by the hour's model amount alone.
pub fn nice_day_cell(temp_f: f32, feels_f: f32, wind_mph: f32, rain_in: f32, rh_pct: f32) -> f32 {
    let some = |v: f32| v.is_finite().then_some(f64::from(v));
    let hour = Hour {
        temp_f: some(temp_f),
        feels_f: some(feels_f),
        wind_mph: some(wind_mph),
        rain_chance: None,
        rain_in: some(rain_in),
        humidity: some(rh_pct),
    };
    score_hour(&hour, Mode::NiceDay).map_or(f32::NAN, |s| s as f32)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn vectors() -> Vec<Value> {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../web/src/shared/testdata/niceDay.vectors.json"
        );
        let text = std::fs::read_to_string(path).expect("niceDay.vectors.json");
        let v: Value = serde_json::from_str(&text).unwrap();
        v["vectors"].as_array().unwrap().clone()
    }

    #[test]
    fn every_shared_vector_gives_the_same_score_and_word() {
        let vectors = vectors();
        assert!(vectors.len() >= 20, "{}", vectors.len());
        for v in vectors {
            let h = &v["hour"];
            let f = |k: &str| h.get(k).and_then(Value::as_f64);
            let hour = Hour {
                temp_f: f("tempF"),
                feels_f: f("feelsF"),
                wind_mph: f("windMph"),
                rain_chance: f("rainChance"),
                rain_in: f("rainIn"),
                humidity: f("humidity"),
            };
            let mode = match v["mode"].as_str().unwrap() {
                "niceDay" => Mode::NiceDay,
                "bonfire" => Mode::Bonfire,
                m => panic!("mode {m}"),
            };
            let name = v["name"].as_str().unwrap();
            let got = score_hour(&hour, mode);
            assert_eq!(got, v["score"].as_f64(), "{name}");
            assert_eq!(got.map(word_for), v["word"].as_str(), "{name}");
        }
    }

    #[test]
    fn map_cell_skips_rain_chance_and_marks_unknown_as_nan() {
        assert_eq!(nice_day_cell(72.0, 72.0, 6.0, 0.0, 45.0), 100.0);
        // Feels-like missing falls back to the air temperature.
        assert_eq!(nice_day_cell(70.0, f32::NAN, 6.0, 0.0, 50.0), 100.0);
        assert!(nice_day_cell(f32::NAN, f32::NAN, 6.0, 0.0, 50.0).is_nan());
        assert_eq!(nice_day_cell(58.0, 58.0, 10.0, 0.15, 95.0), 30.0);
    }

    #[test]
    fn words_follow_the_bands() {
        assert_eq!(word_for(100.0), "Great");
        assert_eq!(word_for(80.0), "Great");
        assert_eq!(word_for(79.0), "Good");
        assert_eq!(word_for(40.0), "Fair");
        assert_eq!(word_for(20.0), "Poor");
        assert_eq!(word_for(0.0), "Do Not");
    }
}
