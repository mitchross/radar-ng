use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Health {
    pub active: bool,
    pub newest_chunk_modified: Option<DateTime<Utc>>,
    pub advancing_key: Option<String>,
    pub volume_hint: Option<u16>,
    pub last_advance: Option<DateTime<Utc>>,
    pub low_sweep_end: Option<DateTime<Utc>>,
    pub access_failure: Option<String>,
    pub processing_failure: Option<String>,
}

#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum State {
    Unknown,
    Warming,
    Healthy,
    Late,
    FutureClock,
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub state: State,
    pub eligible: bool,
    pub feed_late: bool,
    pub source_unreachable: bool,
    pub processing_failure: Option<String>,
}

impl Health {
    pub fn observe(&mut self, key: &str, modified: DateTime<Utc>, now: DateTime<Utc>) {
        if self.newest_chunk_modified.is_none_or(|old| modified > old) {
            self.newest_chunk_modified = Some(modified);
            self.advancing_key = Some(key.into());
            self.volume_hint = key
                .split('/')
                .nth(1)
                .and_then(|n| n.parse().ok())
                .filter(|n| (1..=999).contains(n));
            self.last_advance = Some(now);
        }
        self.access_failure = None;
    }

    pub fn complete_low_sweep(
        &mut self,
        end: DateTime<Utc>,
        now: DateTime<Utc>,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(end <= now, "future low sweep");
        self.low_sweep_end = Some(self.low_sweep_end.map_or(end, |old| old.max(end)));
        Ok(())
    }

    pub fn report(&self, now: DateTime<Utc>) -> Report {
        let state = if !self.active {
            State::Unknown
        } else {
            match self.low_sweep_end.map(|end| now.signed_duration_since(end)) {
                None => State::Warming,
                Some(age) if age < chrono::TimeDelta::zero() => State::FutureClock,
                Some(age) if age <= chrono::TimeDelta::seconds(600) => State::Healthy,
                Some(_) => State::Late,
            }
        };
        Report {
            eligible: state == State::Healthy,
            state,
            feed_late: self.active
                && (self
                    .last_advance
                    .is_none_or(|t| now - t > chrono::TimeDelta::seconds(120))
                    || self
                        .newest_chunk_modified
                        .is_some_and(|t| t > now || now - t > chrono::TimeDelta::seconds(120))),
            source_unreachable: self.access_failure.is_some(),
            processing_failure: self.processing_failure.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeDelta;
    #[test]
    fn future_low_sweep_cannot_poison_last_good_clock() {
        let now = Utc::now();
        let mut h = Health {
            active: true,
            ..Health::default()
        };
        let previous = now - TimeDelta::seconds(800);
        h.complete_low_sweep(previous, now).unwrap();
        assert!(
            h.complete_low_sweep(now + TimeDelta::nanoseconds(1), now)
                .is_err()
        );
        assert_eq!(h.low_sweep_end, Some(previous));
        h.complete_low_sweep(now - TimeDelta::seconds(1), now)
            .unwrap();
        assert!(h.report(now).eligible);
    }
    #[test]
    fn clocks_are_independent_at_boundaries() {
        let now = Utc::now();
        let mut h = Health::default();
        assert_eq!(h.report(now).state, State::Unknown);
        h.active = true;
        assert_eq!(h.report(now).state, State::Warming);
        h.low_sweep_end = Some(now - TimeDelta::seconds(600));
        assert!(h.report(now).eligible);
        assert!(!h.report(now + TimeDelta::nanoseconds(1)).eligible);
        h.observe("higher-tilt", now, now);
        assert!(!h.report(now + TimeDelta::nanoseconds(1)).eligible);
        assert!(!h.report(now + TimeDelta::seconds(120)).feed_late);
        assert!(
            h.report(now + TimeDelta::seconds(120) + TimeDelta::nanoseconds(1))
                .feed_late
        );
        h.low_sweep_end = Some(now + TimeDelta::nanoseconds(1));
        assert_eq!(h.report(now).state, State::FutureClock);
        h.low_sweep_end = Some(now);
        h.access_failure = Some("GET failed".into());
        assert!(h.report(now).eligible && h.report(now).source_unreachable);
        h.observe(
            "older",
            now - TimeDelta::seconds(1),
            now + TimeDelta::seconds(5),
        );
        assert_eq!(h.last_advance, Some(now));
        assert!(!h.report(now).source_unreachable);
        h.observe("future chunk", now + TimeDelta::seconds(1), now);
        assert!(h.report(now).eligible && h.report(now).feed_late);
        h.active = false;
        assert_eq!(h.report(now + TimeDelta::days(1)).state, State::Unknown);
        assert!(!h.report(now + TimeDelta::days(1)).feed_late);
    }
}
