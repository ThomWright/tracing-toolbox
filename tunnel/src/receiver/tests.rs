//! Tests for tracing event receiver.

use assert_matches::assert_matches;

use std::borrow::Cow;

use super::*;
use crate::{CallSiteKind, TracingLevel};

const CALL_SITE_DATA: CallSiteData = create_call_site(Vec::new());

const fn create_call_site(fields: Vec<Cow<'static, str>>) -> CallSiteData {
    CallSiteData {
        kind: CallSiteKind::Span,
        name: Cow::Borrowed("test"),
        target: Cow::Borrowed("tracing_tunnel"),
        level: TracingLevel::Error,
        module_path: Some(Cow::Borrowed("receiver::tests")),
        file: Some(Cow::Borrowed("tests")),
        line: Some(42),
        fields,
    }
}

#[test]
fn unknown_metadata_error() {
    let event = TracingEvent::NewSpan {
        id: 0,
        parent_id: None,
        metadata_id: 0,
        values: TracedValues::new(),
    };
    let mut spans = PersistedSpans::default();
    let mut local_spans = LocalSpans::default();
    let mut receiver =
        TracingEventReceiver::new(PersistedMetadata::default(), &mut spans, &mut local_spans);
    let err = receiver.try_receive(event).unwrap_err();
    assert_matches!(err, ReceiveError::UnknownMetadataId(0));
}

#[test]
fn unknown_span_errors() {
    let bogus_events = [
        TracingEvent::SpanEntered { id: 1 },
        TracingEvent::SpanExited { id: 1 },
        TracingEvent::SpanDropped { id: 1 },
        TracingEvent::NewSpan {
            id: 42,
            parent_id: Some(1),
            metadata_id: 0,
            values: TracedValues::new(),
        },
        TracingEvent::NewEvent {
            metadata_id: 0,
            parent: Some(1),
            values: TracedValues::new(),
        },
        TracingEvent::ValuesRecorded {
            id: 1,
            values: TracedValues::new(),
        },
    ];

    let mut spans = PersistedSpans::default();
    let mut local_spans = LocalSpans::default();
    let mut receiver =
        TracingEventReceiver::new(PersistedMetadata::default(), &mut spans, &mut local_spans);
    receiver.receive(TracingEvent::NewCallSite {
        id: 0,
        data: CALL_SITE_DATA,
    });
    for bogus_event in bogus_events {
        let err = receiver.try_receive(bogus_event).unwrap_err();
        assert_matches!(err, ReceiveError::UnknownSpanId(1));
    }
}

#[test]
fn spans_with_allowed_value_lengths() {
    for values_len in 0..=32 {
        println!("values length: {values_len}");

        let mut spans = PersistedSpans::default();
        let mut local_spans = LocalSpans::default();
        let mut receiver =
            TracingEventReceiver::new(PersistedMetadata::default(), &mut spans, &mut local_spans);
        let fields = (0..values_len)
            .map(|i| Cow::Owned(format!("field{i}")))
            .collect();
        receiver.receive(TracingEvent::NewCallSite {
            id: 0,
            data: create_call_site(fields),
        });

        let values = (0..values_len)
            .map(|i| (format!("field{i}"), TracedValue::Int(i.into())))
            .collect();
        receiver.receive(TracingEvent::NewSpan {
            id: 0,
            parent_id: None,
            metadata_id: 0,
            values,
        });
        receiver.receive(TracingEvent::SpanDropped { id: 0 });
    }
}

#[test]
fn too_many_values_error() {
    let mut spans = PersistedSpans::default();
    let mut local_spans = LocalSpans::default();
    let mut receiver =
        TracingEventReceiver::new(PersistedMetadata::default(), &mut spans, &mut local_spans);
    receiver.receive(TracingEvent::NewCallSite {
        id: 0,
        data: CALL_SITE_DATA,
    });

    let values = (0..33)
        .map(|i| (format!("field{i}"), TracedValue::Int(i.into())))
        .collect();
    let bogus_event = TracingEvent::NewSpan {
        id: 0,
        parent_id: None,
        metadata_id: 0,
        values,
    };
    let err = receiver.try_receive(bogus_event).unwrap_err();
    assert_matches!(
        err,
        ReceiveError::TooManyValues {
            actual: 33,
            max: 32
        }
    );
}

#[test]
fn receiver_does_not_panic_on_bogus_field() {
    let events = [
        TracingEvent::NewCallSite {
            id: 0,
            data: CALL_SITE_DATA,
        },
        TracingEvent::NewSpan {
            id: 0,
            parent_id: None,
            metadata_id: 0,
            values: TracedValues::from_iter([("i".to_owned(), TracedValue::from(42_i64))]),
        },
    ];

    let mut spans = PersistedSpans::default();
    let mut local_spans = LocalSpans::default();
    let mut receiver =
        TracingEventReceiver::new(PersistedMetadata::default(), &mut spans, &mut local_spans);
    for event in events {
        receiver.receive(event);
    }
}

#[test]
fn restoring_spans() {
    let metadata = PersistedMetadata {
        inner: HashMap::from_iter([(0, CALL_SITE_DATA)]),
    };
    let mut spans = PersistedSpans {
        inner: HashMap::from_iter([(
            1,
            SpanData {
                metadata_id: 0,
                parent_id: None,
                ref_count: 1,
                values: TracedValues::new(),
            },
        )]),
    };
    let mut local_spans = LocalSpans::default();

    let mut receiver = TracingEventReceiver::new(metadata, &mut spans, &mut local_spans);
    visit_and_drop_span(&mut receiver);
}

fn visit_and_drop_span(receiver: &mut TracingEventReceiver<'_>) {
    receiver.receive(TracingEvent::SpanEntered { id: 1 });
    assert!(receiver.local_spans.inner.contains_key(&1));

    receiver.receive(TracingEvent::SpanExited { id: 1 });
    receiver.receive(TracingEvent::SpanDropped { id: 1 });
    assert!(!receiver.spans.inner.contains_key(&1));
    assert!(!receiver.local_spans.inner.contains_key(&1));
}

#[test]
fn restoring_span_after_recording_values() {
    let call_site = create_call_site(vec!["i".into()]);
    let metadata = PersistedMetadata {
        inner: HashMap::from_iter([(0, call_site)]),
    };
    let mut spans = PersistedSpans {
        inner: HashMap::from_iter([(
            1,
            SpanData {
                metadata_id: 0,
                parent_id: None,
                ref_count: 1,
                values: TracedValues::new(),
            },
        )]),
    };
    let mut local_spans = LocalSpans::default();

    let mut receiver = TracingEventReceiver::new(metadata, &mut spans, &mut local_spans);
    receiver.receive(TracingEvent::ValuesRecorded {
        id: 1,
        values: TracedValues::from_iter([("i".to_owned(), TracedValue::from(42_i64))]),
    });
    assert_eq!(receiver.spans.inner[&1].values["i"], 42_i64);
    assert!(!receiver.local_spans.inner.contains_key(&1));

    visit_and_drop_span(&mut receiver);
}
