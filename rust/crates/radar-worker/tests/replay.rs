//! Offline determinism gate: replay every recorded Python workflow history with the Rust workflow code.
//!
//! Fixtures live at `testdata/replay_fixtures/<WorkflowType>/<version>/<scenario>.json[.gz]`. They are
//! copied unchanged from `temporal/workflows/replay_fixtures` in the Python repo, so this crate's tests are
//! self-contained. `version` is the Python worker release that produced the history (`v1.1.49` is the
//! build deployed at cutover), or `src-<sha>` for an unreleased checkout.
//!
//! This is the Rust side of `temporal/workflows/test_replay.py`:
//! - the fixture tree must cover exactly the registered workflow types;
//! - every release must keep the [`REQUIRED_SCENARIOS`], and each such history must show the path it is
//!   named for (an event type, or an activity type that was scheduled);
//! - every history must replay without a nondeterminism error or workflow-task failure against the
//!   workflows registered by `radar worker` ([`radar_worker::worker::register_workflows`]), once per role
//!   that registers its type.
//!
//! No Temporal server is needed. A fixture that is known not to replay must be listed in [`ALLOWLIST`] with
//! the reason; an allowlisted fixture that starts replaying fails the test, so the list cannot go stale.
//!
//! Run with `cargo test -p radar-worker --test replay -- --nocapture` to see the per-fixture table.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use prost::Message;
use prost_reflect::{DescriptorPool, DynamicMessage};

use radar_types::wire::WorkerRole;
use radar_worker::registration;
use radar_worker::worker::register_workflows;
use temporalio_client::WorkflowHistory;
use temporalio_common::protos::temporal::api::enums::v1::EventType;
use temporalio_common::protos::temporal::api::history::v1::History;
use temporalio_common::protos::temporal::api::history::v1::history_event::Attributes;
use temporalio_sdk::workflow_replayer::{WorkflowReplayer, WorkflowReplayerOptions};

/// What proves a scenario took its path: an event type, or the name of an activity that was scheduled.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Marker {
    Event(EventType),
    Activity(&'static str),
}

use Marker::{Activity, Event};

const COMPLETED: Marker = Event(EventType::WorkflowExecutionCompleted);
const FAILED: Marker = Event(EventType::WorkflowExecutionFailed);
const ACTIVITY_FAILED: Marker = Event(EventType::ActivityTaskFailed);
const SIGNALED: Marker = Event(EventType::WorkflowExecutionSignaled);
const TIMER_FIRED: Marker = Event(EventType::TimerFired);
const CONTINUED: Marker = Event(EventType::WorkflowExecutionContinuedAsNew);

type Scenarios = &'static [(&'static str, &'static [Marker])];

const SINGLE_ACTIVITY: Scenarios = &[
    ("success", &[COMPLETED]),
    ("activity-error", &[ACTIVITY_FAILED, FAILED]),
];

/// Copied from `REQUIRED_SCENARIOS` in `test_replay.py`. Every workflow must retain these scenarios in every
/// release directory, each with proof it took the path it is named for. Add, do not remove.
const REQUIRED_SCENARIOS: &[(&str, Scenarios)] = &[
    ("RadarDeploymentSmokeWorkflow", SINGLE_ACTIVITY),
    (
        "IngestMrmsWorkflow",
        &[
            ("success", &[COMPLETED, Activity("mrms_mark_processed")]),
            ("partial", &[COMPLETED]),
            ("frame-activity-error", &[ACTIVITY_FAILED, COMPLETED]),
            ("list-activity-error", &[ACTIVITY_FAILED, FAILED]),
        ],
    ),
    (
        "IngestHrrrWorkflow",
        &[
            ("success", &[COMPLETED, Activity("hrrr_mark_processed")]),
            ("partial", &[COMPLETED, Activity("hrrr_publish_run")]),
            ("hour-activity-error", &[ACTIVITY_FAILED, COMPLETED]),
            ("find-activity-error", &[ACTIVITY_FAILED, FAILED]),
        ],
    ),
    (
        "IngestAirQualityWorkflow",
        &[
            ("success", &[COMPLETED, Activity("aqm_mark_processed")]),
            ("partial", &[COMPLETED, Activity("aqm_publish_run")]),
            ("chunk-activity-error", &[ACTIVITY_FAILED, COMPLETED]),
            ("find-activity-error", &[ACTIVITY_FAILED, FAILED]),
        ],
    ),
    (
        "PollAlertsWorkflow",
        &[
            (
                "geometry-fanout",
                &[COMPLETED, Activity("signal_matching_storm_watches")],
            ),
            ("signal-activity-error", &[ACTIVITY_FAILED, COMPLETED]),
            ("fetch-activity-error", &[ACTIVITY_FAILED, FAILED]),
        ],
    ),
    (
        "WatchStormWorkflow",
        &[
            ("signal-unpin", &[SIGNALED, COMPLETED]),
            ("timer-poll", &[TIMER_FIRED, COMPLETED]),
            (
                "push-change",
                &[
                    Activity("detect_storm_change"),
                    Activity("fan_out_push_to_user"),
                    COMPLETED,
                ],
            ),
            (
                "alert-signal-push",
                &[SIGNALED, Activity("fan_out_push_to_user"), COMPLETED],
            ),
            ("push-activity-error", &[ACTIVITY_FAILED, COMPLETED]),
            ("compare-activity-error", &[ACTIVITY_FAILED, FAILED]),
            ("continue-as-new", &[CONTINUED]),
        ],
    ),
    ("IngestLightningWorkflow", SINGLE_ACTIVITY),
    ("IngestTropicalWorkflow", SINGLE_ACTIVITY),
    ("NowcastWorkflow", SINGLE_ACTIVITY),
    ("TileCleanupWorkflow", SINGLE_ACTIVITY),
    ("RegisterPushTokenWorkflow", SINGLE_ACTIVITY),
    ("DeletePushTokenWorkflow", SINGLE_ACTIVITY),
    ("OpenMeteoSyncWorkflow", SINGLE_ACTIVITY),
];

/// Fixtures that are known not to replay on the Rust code, as `(label, reason)`. A label is
/// `<WorkflowType>/<version>/<scenario>`. Each entry is a documented, deliberate incompatibility (see the
/// README "Replay" section); an entry whose fixture replays cleanly fails the test.
const ALLOWLIST: &[(&str, &str)] = &[];

/// One recorded history.
struct Fixture {
    workflow: String,
    version: String,
    scenario: String,
    history: History,
}

impl Fixture {
    fn label(&self) -> String {
        format!("{}/{}/{}", self.workflow, self.version, self.scenario)
    }

    /// Event types present, plus the names of every scheduled activity.
    fn markers(&self) -> BTreeSet<Marker> {
        let mut found = BTreeSet::new();
        for event in &self.history.events {
            if let Ok(kind) = EventType::try_from(event.event_type) {
                found.insert(Event(kind));
            }
            // Only activity names that some required scenario asks about are recorded.
            if let Some(Attributes::ActivityTaskScheduledEventAttributes(attrs)) = &event.attributes
                && let Some(kind) = &attrs.activity_type
                && let Some(name) = activity_marker_name(&kind.name)
            {
                found.insert(Activity(name));
            }
        }
        found
    }

    fn started_workflow_type(&self) -> Option<&str> {
        match self.history.events.first()?.attributes.as_ref()? {
            Attributes::WorkflowExecutionStartedEventAttributes(attrs) => {
                attrs.workflow_type.as_ref().map(|t| t.name.as_str())
            }
            _ => None,
        }
    }
}

/// The `&'static` activity name a required scenario uses, if `name` is one of them.
fn activity_marker_name(name: &str) -> Option<&'static str> {
    REQUIRED_SCENARIOS
        .iter()
        .flat_map(|(_, scenarios)| scenarios.iter())
        .flat_map(|(_, markers)| markers.iter())
        .find_map(|marker| match marker {
            Activity(wanted) if *wanted == name => Some(*wanted),
            _ => None,
        })
}

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata/replay_fixtures")
}

fn sorted_entries(dir: &Path) -> Vec<PathBuf> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("reading {}: {e}", dir.display()))
        .map(|entry| entry.expect("directory entry").path())
        .collect();
    entries.sort();
    entries
}

/// Load every `<WorkflowType>/<version>/<scenario>.json[.gz]` under the fixture directory.
fn load_fixtures() -> Vec<Fixture> {
    let mut fixtures = Vec::new();
    for workflow_dir in sorted_entries(&fixture_dir()) {
        if !workflow_dir.is_dir() {
            continue; // README.md, generate.py
        }
        let workflow = file_name(&workflow_dir);
        for version_dir in sorted_entries(&workflow_dir) {
            let version = file_name(&version_dir);
            for path in sorted_entries(&version_dir) {
                let name = file_name(&path);
                let (scenario, gz) = if let Some(stem) = name.strip_suffix(".json.gz") {
                    (stem.to_owned(), true)
                } else if let Some(stem) = name.strip_suffix(".json") {
                    (stem.to_owned(), false)
                } else {
                    continue;
                };
                let raw = std::fs::read(&path)
                    .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
                let bytes = if gz {
                    let mut out = Vec::new();
                    flate2::read::GzDecoder::new(raw.as_slice())
                        .read_to_end(&mut out)
                        .unwrap_or_else(|e| panic!("gunzip {}: {e}", path.display()));
                    out
                } else {
                    raw
                };
                let history = decode_history(&bytes)
                    .unwrap_or_else(|e| panic!("decoding {}: {e}", path.display()));
                fixtures.push(Fixture {
                    workflow: workflow.clone(),
                    version: version.clone(),
                    scenario,
                    history,
                });
            }
        }
    }
    fixtures
}

/// The Temporal API descriptor set compiled by `temporalio-protos` (see `build.rs`), so the JSON is read
/// with exactly the proto schema the SDK uses.
fn descriptor_pool() -> &'static DescriptorPool {
    static POOL: OnceLock<DescriptorPool> = OnceLock::new();
    POOL.get_or_init(|| {
        DescriptorPool::decode(include_bytes!(env!("TEMPORAL_PROTO_DESCRIPTORS")).as_slice())
            .expect("temporalio-protos descriptor set decodes")
    })
}

/// Decode a proto3-JSON history export (`{"events": [...]}` with camelCase fields, enum names, base64 payload
/// `data`, int64 as strings) into the SDK's `History`. The SDK's `WorkflowHistory::from_json` cannot be used:
/// in temporalio-protos 0.10 the history messages derive plain serde (snake_case fields, numeric enums), not
/// proto3 JSON. The JSON goes through a reflective message and is re-decoded from the binary encoding, which
/// is unambiguous. Unknown fields are rejected, so a schema drift between the Python and Rust SDKs shows up
/// here instead of being dropped silently.
fn decode_history(bytes: &[u8]) -> anyhow::Result<History> {
    let descriptor = descriptor_pool()
        .get_message_by_name("temporal.api.history.v1.History")
        .ok_or_else(|| anyhow::anyhow!("History missing from descriptor set"))?;
    let mut json = serde_json::Deserializer::from_slice(bytes);
    let message = DynamicMessage::deserialize(descriptor, &mut json)?;
    json.end()?;
    Ok(History::decode(message.encode_to_vec().as_slice())?)
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_else(|| panic!("non-UTF-8 path {}", path.display()))
        .to_owned()
}

/// `v<major>.<minor>.<patch>` or `src-<7..40 lowercase hex>`, as `VERSION_RE` in `test_replay.py`.
fn version_is_valid(version: &str) -> bool {
    if let Some(rest) = version.strip_prefix('v') {
        let parts: Vec<&str> = rest.split('.').collect();
        return parts.len() == 3
            && parts
                .iter()
                .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    }
    if let Some(sha) = version.strip_prefix("src-") {
        return (7..=40).contains(&sha.len())
            && sha.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
    }
    false
}

fn required(workflow: &str) -> Scenarios {
    REQUIRED_SCENARIOS
        .iter()
        .find(|(name, _)| *name == workflow)
        .map(|(_, scenarios)| *scenarios)
        .unwrap_or_else(|| panic!("{workflow} has no REQUIRED_SCENARIOS entry"))
}

#[test]
fn fixture_tree_matches_registry() {
    let fixtures = load_fixtures();
    assert!(
        !fixtures.is_empty(),
        "no fixtures under {:?}",
        fixture_dir()
    );
    let registered: BTreeSet<&str> = registration(WorkerRole::All)
        .workflows
        .into_iter()
        .collect();
    let in_tree: BTreeSet<&str> = fixtures.iter().map(|f| f.workflow.as_str()).collect();
    let required: BTreeSet<&str> = REQUIRED_SCENARIOS.iter().map(|(name, _)| *name).collect();
    assert_eq!(in_tree, registered, "fixture tree vs registered workflows");
    assert_eq!(
        required, registered,
        "REQUIRED_SCENARIOS vs registered workflows"
    );

    let mut labels = BTreeSet::new();
    for fixture in &fixtures {
        let label = fixture.label();
        assert!(labels.insert(label.clone()), "duplicate fixture {label}");
        assert!(
            version_is_valid(&fixture.version),
            "{label}: bad version dir"
        );
        assert!(!fixture.history.events.is_empty(), "{label}: no events");
        assert_eq!(
            fixture.started_workflow_type(),
            Some(fixture.workflow.as_str()),
            "{label}: first event must start the directory's workflow type"
        );
    }
    for (label, _) in ALLOWLIST {
        assert!(
            labels.contains(*label),
            "allowlist names a missing fixture {label}"
        );
    }
}

#[test]
fn required_scenarios_are_retained_and_prove_their_path() {
    let fixtures = load_fixtures();
    let mut by_release: BTreeMap<(&str, &str), Vec<&Fixture>> = BTreeMap::new();
    for fixture in &fixtures {
        by_release
            .entry((&fixture.workflow, &fixture.version))
            .or_default()
            .push(fixture);
    }
    let mut problems = Vec::new();
    for ((workflow, version), fixtures) in &by_release {
        let scenarios = required(workflow);
        let present: BTreeSet<&str> = fixtures.iter().map(|f| f.scenario.as_str()).collect();
        for (scenario, _) in scenarios {
            if !present.contains(scenario) {
                problems.push(format!(
                    "{workflow}/{version}: missing required scenario {scenario}"
                ));
            }
        }
        for fixture in fixtures {
            let Some((_, markers)) = scenarios.iter().find(|(s, _)| *s == fixture.scenario) else {
                continue;
            };
            let found = fixture.markers();
            let missing: Vec<&Marker> = markers.iter().filter(|m| !found.contains(m)).collect();
            if !missing.is_empty() {
                problems.push(format!("{}: missing markers {missing:?}", fixture.label()));
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// Replayers for every role that registers at least one workflow, built with the worker's own registration.
fn role_replayers() -> Vec<(WorkerRole, BTreeSet<&'static str>, WorkflowReplayer)> {
    WorkerRole::ALL
        .into_iter()
        .filter_map(|role| {
            let workflows: BTreeSet<&'static str> =
                registration(role).workflows.into_iter().collect();
            if workflows.is_empty() {
                return None; // open-meteo serves activities only
            }
            let mut options = WorkflowReplayerOptions::new().build();
            register_workflows(&mut options, role)
                .unwrap_or_else(|e| panic!("registering {role:?}: {e:#}"));
            let replayer = WorkflowReplayer::new(options)
                .unwrap_or_else(|e| panic!("replayer for {role:?}: {e}"));
            Some((role, workflows, replayer))
        })
        .collect()
}

#[tokio::test(flavor = "current_thread")]
async fn every_fixture_replays() {
    let fixtures = load_fixtures();
    let replayers = role_replayers();
    let allowlist: BTreeMap<&str, &str> = ALLOWLIST.iter().copied().collect();

    // label -> failure messages (one per role that failed), empty when every role replayed it.
    let mut outcomes: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for fixture in &fixtures {
        let mut failures = Vec::new();
        let mut roles = 0;
        for (role, workflows, replayer) in &replayers {
            if !workflows.contains(fixture.workflow.as_str()) {
                continue;
            }
            roles += 1;
            let history = WorkflowHistory::from(fixture.history.clone());
            if let Err(error) = replayer.replay_workflow(history).await {
                failures.push(format!("[{role:?}] {error}"));
            }
        }
        assert!(
            roles >= 2,
            "{}: registered by fewer than two roles",
            fixture.label()
        );
        outcomes.insert(fixture.label(), failures);
    }

    // Per-fixture report and per-workflow, per-version tally.
    let mut tally: BTreeMap<(String, String), [usize; 3]> = BTreeMap::new();
    let mut problems = Vec::new();
    let mut report = String::from("\nreplay results:\n");
    for fixture in &fixtures {
        let label = fixture.label();
        let failures = &outcomes[&label];
        let allowed = allowlist.get(label.as_str());
        let (status, column) = match (failures.is_empty(), allowed) {
            (true, None) => ("PASS", 0),
            (false, None) => {
                problems.push(format!("{label}: {}", failures.join("; ")));
                ("FAIL", 1)
            }
            (false, Some(_)) => ("ALLOWLISTED", 2),
            (true, Some(_)) => {
                problems.push(format!(
                    "{label}: allowlisted but replays cleanly; remove it from ALLOWLIST"
                ));
                ("STALE-ALLOW", 1)
            }
        };
        tally
            .entry((fixture.workflow.clone(), fixture.version.clone()))
            .or_default()[column] += 1;
        report.push_str(&format!("  {status:<11} {label}\n"));
        for failure in failures {
            let first_line = failure.lines().next().unwrap_or_default();
            report.push_str(&format!("              {first_line}\n"));
        }
    }
    report.push_str("\nby workflow and version (pass / fail / allowlisted):\n");
    for ((workflow, version), [pass, fail, allowed]) in &tally {
        report.push_str(&format!(
            "  {workflow:<30} {version:<12} {pass:>2} / {fail:>2} / {allowed:>2}\n"
        ));
    }
    println!("{report}");
    assert!(
        problems.is_empty(),
        "{} fixture(s) failed replay (table above):\n{}",
        problems.len(),
        problems.join("\n")
    );
}

/// The gate is not vacuous: a history whose recorded command differs from what the code issues must fail.
#[tokio::test(flavor = "current_thread")]
async fn a_tampered_history_is_reported_as_nondeterministic() {
    let fixture = load_fixtures()
        .into_iter()
        .find(|f| f.label() == "NowcastWorkflow/v1.1.49/success")
        .expect("NowcastWorkflow/v1.1.49/success fixture");
    let mut history = fixture.history;
    let mut renamed = 0;
    for event in &mut history.events {
        if let Some(Attributes::ActivityTaskScheduledEventAttributes(attrs)) = &mut event.attributes
            && let Some(kind) = &mut attrs.activity_type
        {
            kind.name = "synthetic_not_the_real_activity".to_owned();
            renamed += 1;
        }
    }
    assert_eq!(renamed, 1, "the success history schedules one activity");

    let mut options = WorkflowReplayerOptions::new().build();
    register_workflows(&mut options, WorkerRole::All).expect("register");
    let replayer = WorkflowReplayer::new(options).expect("replayer");
    let error = replayer
        .replay_workflow(WorkflowHistory::from(history))
        .await
        .expect_err("a renamed activity must not replay");
    assert!(
        error.to_string().contains("nondeterministic"),
        "unexpected error kind: {error}"
    );
}
