//! Workflow definitions. Each module holds one workflow type (names fixed by `radar_types::wire`), its
//! pure decision helpers, and unit tests for them.
//!
//! Determinism: workflow code uses only SDK primitives (`execute_activity`, `timer`, `wait_condition`,
//! `join_all`, `select!`) and the hand-written sliding window in [`bounded`], which polls its futures in
//! input order. No Tokio concurrency primitives appear here.

pub mod airquality;
pub mod deployment_smoke;
pub mod global_model;
pub mod hrrr;
pub mod lightning;
pub mod mrms;
pub mod nowcast;
pub mod open_meteo_sync;
pub mod poll_alerts;
pub mod push_tokens;
pub mod tile_cleanup;
pub mod tropical;
pub mod watch_storm;

use std::future::Future;
use std::pin::Pin;
use std::task::Poll;
use std::time::Duration;

use radar_types::retry::{ActivityCallSpec, RetryPolicySpec};
use temporalio_common::RetryPolicy;
use temporalio_common::data_converters::{
    PayloadConversionError, SerializationContext, TemporalDeserializable, TemporalSerializable,
};
use temporalio_common::protos::temporal::api::common::v1::Payload;
use temporalio_sdk::{ActivityCloseTimeouts, ActivityOptions};

/// Retry policy from the shared constants. Every field is written explicitly (spec 1.3).
pub fn retry(spec: &RetryPolicySpec) -> RetryPolicy {
    RetryPolicy::builder()
        .initial_interval(Duration::from_secs(spec.initial_interval_s))
        .backoff_coefficient(spec.backoff)
        .maximum_interval(Duration::from_secs(spec.max_interval_s))
        .maximum_attempts(spec.max_attempts)
        .build()
}

/// Activity options for one call site. `schedule_to_close` applies only when the spec sets it.
pub fn options(spec: &ActivityCallSpec) -> ActivityOptions {
    let start_to_close = Duration::from_secs(spec.start_to_close_s);
    let timeouts = match spec.schedule_to_close_s {
        Some(schedule_to_close) => ActivityCloseTimeouts::ScheduleAndStartToClose {
            schedule_to_close: Duration::from_secs(schedule_to_close),
            start_to_close,
        },
        None => ActivityCloseTimeouts::StartToClose(start_to_close),
    };
    ActivityOptions::with_close_timeouts(timeouts)
        .maybe_heartbeat_timeout(spec.heartbeat_s.map(Duration::from_secs))
        .retry_policy(retry(&spec.retry))
        .build()
}

/// Same as [`options`], routed to an explicit queue (only `open_meteo_sync` does this).
pub fn options_on_queue(spec: &ActivityCallSpec, queue: &str) -> ActivityOptions {
    let mut opts = options(spec);
    opts.task_queue = Some(queue.to_owned());
    opts
}

/// An optional single workflow argument, as Python's `run(self, args: T | None = None)`.
///
/// Python lets a start (or a schedule) pass no arguments at all, and the workflow then sees `None`. The SDK
/// decodes `Option<T>` only from exactly one payload, so a zero-payload start failed with "Failed to create
/// workflow execution" (replay gate, `IngestMrmsWorkflow/*/empty-backlog`). This wrapper accepts zero
/// payloads, a `binary/null` payload, or one `json/plain` payload (JSON `null` included). It serialises
/// `None` as no payloads and `Some` as one `json/plain` payload. It implements the SDK's payload traits
/// directly (the default converter tries these "wrapper" impls before serde), so it derives no serde traits.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OptionalArg<T>(pub Option<T>);

const ENCODING_KEY: &str = "encoding";
const JSON_PLAIN: &[u8] = b"json/plain";
const BINARY_NULL: &[u8] = b"binary/null";

impl<T: serde::de::DeserializeOwned> TemporalDeserializable for OptionalArg<T> {
    fn from_payload(
        _ctx: &SerializationContext<'_>,
        payload: Payload,
    ) -> Result<Self, PayloadConversionError> {
        match payload.metadata.get(ENCODING_KEY).map(Vec::as_slice) {
            Some(BINARY_NULL) => Ok(Self(None)),
            Some(JSON_PLAIN) => serde_json::from_slice::<Option<T>>(&payload.data)
                .map(Self)
                .map_err(|e| PayloadConversionError::EncodingError(Box::new(e))),
            _ => Err(PayloadConversionError::WrongEncoding),
        }
    }

    fn from_payloads(
        ctx: &SerializationContext<'_>,
        payloads: Vec<Payload>,
    ) -> Result<Self, PayloadConversionError> {
        let mut payloads = payloads.into_iter();
        match (payloads.next(), payloads.next()) {
            (None, _) => Ok(Self(None)),
            (Some(payload), None) => Self::from_payload(ctx, payload),
            (Some(_), Some(_)) => Err(PayloadConversionError::WrongEncoding),
        }
    }
}

impl<T: serde::Serialize> TemporalSerializable for OptionalArg<T> {
    fn to_payload(
        &self,
        _ctx: &SerializationContext<'_>,
    ) -> Result<Payload, PayloadConversionError> {
        let data = serde_json::to_vec(&self.0)
            .map_err(|e| PayloadConversionError::EncodingError(Box::new(e)))?;
        Ok(Payload {
            metadata: [(ENCODING_KEY.to_owned(), JSON_PLAIN.to_vec())].into(),
            data,
            ..Default::default()
        })
    }

    fn to_payloads(
        &self,
        ctx: &SerializationContext<'_>,
    ) -> Result<Vec<Payload>, PayloadConversionError> {
        match self.0 {
            None => Ok(Vec::new()),
            Some(_) => Ok(vec![self.to_payload(ctx)?]),
        }
    }
}

/// Run `f` over `items` with at most `limit` futures in flight, returning results in input order.
///
/// This is the deterministic replacement for Python `Semaphore(N)` plus `gather` (F3). A slot is freed as soon as
/// ANY in-flight item finishes, so one slow item does not hold the other slots. The old `buffered` version did
/// hold them, because it refilled only after the head item was yielded.
///
/// Determinism and command order (spec 10.2):
/// - The window is a `Vec` kept in input order. Each pass polls the window front to back, and a new item is
///   appended at the back. First polls therefore happen in input order, so activity commands are issued in
///   input order. The first `limit` items are scheduled together, and each later item is scheduled when a
///   slot frees.
/// - Which item finishes first comes from history, so a replay makes the same choices.
/// - No futures-util combinators and no Tokio primitives are used. Each child future is polled with the
///   workflow's own waker, so the SDK's wake handling is the only waker machinery involved. The SDK's
///   `join_all` wraps futures-util's version in `SdkGuardedFuture` for this reason. A hand-written window
///   avoids needing that wrapper, which is crate-private.
///
/// Results are returned in input order regardless of completion order.
pub async fn bounded<I, T, F, Fut>(items: Vec<I>, limit: usize, mut f: F) -> Vec<T>
where
    F: FnMut(I) -> Fut,
    Fut: Future<Output = T>,
{
    let limit = limit.max(1);
    let total = items.len();
    let mut pending_items = items.into_iter().enumerate();
    // (input index, future), in input order.
    let mut window: Vec<(usize, Pin<Box<Fut>>)> = Vec::with_capacity(limit);
    let mut results: Vec<Option<T>> = std::iter::repeat_with(|| None).take(total).collect();
    std::future::poll_fn(|cx| {
        loop {
            while window.len() < limit {
                match pending_items.next() {
                    Some((index, item)) => window.push((index, Box::pin(f(item)))),
                    None => break,
                }
            }
            if window.is_empty() {
                return Poll::Ready(());
            }
            let mut finished = None;
            for (slot, (_, fut)) in window.iter_mut().enumerate() {
                if let Poll::Ready(value) = fut.as_mut().poll(cx) {
                    finished = Some((slot, value));
                    break;
                }
            }
            match finished {
                // Free the slot, then loop: the refill starts the next item before anything else is polled.
                Some((slot, value)) => {
                    let (index, _) = window.remove(slot);
                    results[index] = Some(value);
                }
                None => return Poll::Pending,
            }
        }
    })
    .await;
    results
        .into_iter()
        .map(|r| r.expect("every item completes before bounded returns"))
        .collect()
}

#[cfg(test)]
mod bounded_tests {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::task::Poll;

    use super::bounded;

    /// Pending once and wakes itself, so other window members are polled in between.
    async fn yield_once() {
        let mut yielded = false;
        std::future::poll_fn(|cx| {
            if yielded {
                Poll::Ready(())
            } else {
                yielded = true;
                cx.waker().wake_by_ref();
                Poll::Pending
            }
        })
        .await;
    }

    #[test]
    fn a_freed_slot_starts_the_next_item_while_a_slow_head_is_pending() {
        // F3 regression. Item 0 waits until item 2 has started. With `buffered`, item 2 cannot start while
        // item 0 is pending, so item 0 gives up after its spin budget and the assertions fail. They do not
        // hang.
        let log: Rc<RefCell<Vec<String>>> = Rc::default();
        let item2_started = Rc::new(Cell::new(false));
        let body = {
            let log = log.clone();
            let item2_started = item2_started.clone();
            move |i: usize| {
                let log = log.clone();
                let item2_started = item2_started.clone();
                async move {
                    log.borrow_mut().push(format!("start {i}"));
                    match i {
                        0 => {
                            let mut spins = 0u32;
                            std::future::poll_fn(|cx| {
                                if item2_started.get() || spins >= 1_000 {
                                    Poll::Ready(())
                                } else {
                                    spins += 1;
                                    cx.waker().wake_by_ref();
                                    Poll::Pending
                                }
                            })
                            .await;
                            let note = if item2_started.get() {
                                "released 0"
                            } else {
                                "gave up 0"
                            };
                            log.borrow_mut().push(note.to_string());
                        }
                        2 => item2_started.set(true),
                        _ => {}
                    }
                    i
                }
            }
        };
        let results = futures::executor::block_on(bounded(vec![0usize, 1, 2, 3], 2, body));
        assert_eq!(results, vec![0, 1, 2, 3], "results are in input order");
        let log = log.borrow();
        assert!(
            !log.iter().any(|e| e == "gave up 0"),
            "item 2 never started while item 0 was pending: {log:?}"
        );
        let starts: Vec<&str> = log
            .iter()
            .filter(|e| e.starts_with("start "))
            .map(String::as_str)
            .collect();
        assert_eq!(
            starts,
            ["start 0", "start 1", "start 2", "start 3"],
            "activity starts are in input order"
        );
    }

    #[test]
    fn in_flight_never_exceeds_the_limit_and_results_stay_ordered() {
        let in_flight = Rc::new(Cell::new(0usize));
        let max_seen = Rc::new(Cell::new(0usize));
        let body = {
            let in_flight = in_flight.clone();
            let max_seen = max_seen.clone();
            move |i: usize| {
                let in_flight = in_flight.clone();
                let max_seen = max_seen.clone();
                async move {
                    in_flight.set(in_flight.get() + 1);
                    max_seen.set(max_seen.get().max(in_flight.get()));
                    // Uneven durations, so completions arrive out of input order.
                    for _ in 0..(i % 3 + 1) {
                        yield_once().await;
                    }
                    in_flight.set(in_flight.get() - 1);
                    i * 10
                }
            }
        };
        let results =
            futures::executor::block_on(bounded((0..10usize).collect::<Vec<_>>(), 3, body));
        assert_eq!(results, (0..10).map(|i| i * 10).collect::<Vec<_>>());
        assert_eq!(
            max_seen.get(),
            3,
            "the window fills to the limit and no further"
        );
        assert_eq!(in_flight.get(), 0);
    }

    #[test]
    fn empty_input_and_zero_limit_are_handled() {
        let empty = futures::executor::block_on(bounded(Vec::<u8>::new(), 4, |x| async move { x }));
        assert!(empty.is_empty());
        let clamped = futures::executor::block_on(bounded(vec![1u8, 2], 0, |x| async move { x }));
        assert_eq!(clamped, vec![1, 2], "a zero limit is treated as one");
    }
}

#[cfg(test)]
mod optional_arg_tests {
    use radar_types::payloads::mrms::IngestMrmsArgs;
    use temporalio_common::data_converters::{
        GenericPayloadConverter, PayloadConverter, SerializationContext, SerializationContextData,
    };
    use temporalio_common::protos::temporal::api::common::v1::Payload;

    use super::OptionalArg;

    fn payload(encoding: &str, data: &[u8]) -> Payload {
        Payload {
            metadata: [("encoding".to_owned(), encoding.as_bytes().to_vec())].into(),
            data: data.to_vec(),
            ..Default::default()
        }
    }

    /// Decode through the SDK's default converter, as the worker does for a workflow input.
    fn decode(payloads: Vec<Payload>) -> OptionalArg<IngestMrmsArgs> {
        let converter = PayloadConverter::default();
        let data = SerializationContextData::None;
        let ctx = SerializationContext::new(&data, &converter);
        converter.from_payloads(&ctx, payloads).expect("decodes")
    }

    #[test]
    fn a_start_without_arguments_is_none() {
        assert_eq!(decode(Vec::new()), OptionalArg(None));
        assert_eq!(decode(vec![payload("binary/null", b"")]), OptionalArg(None));
        assert_eq!(
            decode(vec![payload("json/plain", b"null")]),
            OptionalArg(None)
        );
    }

    #[test]
    fn a_python_json_argument_is_some() {
        let decoded = decode(vec![payload(
            "json/plain",
            br#"{"layer_name":"synthetic-layer","mrms_prefix":"synthetic-prefix"}"#,
        )]);
        let args = decoded.0.expect("some");
        assert_eq!(args.layer_name, "synthetic-layer");
        assert_eq!(args.mrms_prefix, "synthetic-prefix");
    }

    #[test]
    fn encoding_round_trips_through_the_default_converter() {
        let converter = PayloadConverter::default();
        let data = SerializationContextData::None;
        let ctx = SerializationContext::new(&data, &converter);
        let none = converter
            .to_payloads(&ctx, &OptionalArg::<IngestMrmsArgs>(None))
            .expect("encodes");
        assert!(
            none.is_empty(),
            "None is sent as no arguments, as Python does"
        );
        let some = OptionalArg(Some(IngestMrmsArgs::default()));
        let encoded = converter.to_payloads(&ctx, &some).expect("encodes");
        assert_eq!(encoded.len(), 1);
        assert_eq!(decode(encoded), some);
    }
}
