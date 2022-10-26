//! Integration tests for Tardigrade tracing infrastructure.

use assert_matches::assert_matches;
use insta::assert_yaml_snapshot;
use once_cell::sync::Lazy;
use tracing_core::{Level, Subscriber};
use tracing_subscriber::{registry::LookupSpan, FmtSubscriber};

use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
};

mod fib;

use tracing_tunnel::{
    CallSiteKind, PersistedMetadata, PersistedSpans, TracedValue, TracingEvent,
    TracingEventReceiver, TracingLevel,
};

#[derive(Debug)]
struct RecordedEvents {
    short: Vec<TracingEvent>,
    long: Vec<TracingEvent>,
}

static EVENTS: Lazy<RecordedEvents> = Lazy::new(|| RecordedEvents {
    short: fib::record_events(5),
    long: fib::record_events(80),
});

#[test]
fn event_snapshot() {
    let mut events = EVENTS.short.clone();
    for event in &mut events {
        if let TracingEvent::NewCallSite { data, .. } = event {
            // Make event data not depend on specific lines, which could easily
            // change due to refactoring etc.
            data.line = Some(42);
            if matches!(data.kind, CallSiteKind::Event) {
                data.name = Cow::Borrowed("event");
            }
        }
    }
    assert_yaml_snapshot!("events-fib-5", events);
}

#[test]
fn resource_management_for_tracing_events() {
    let events = &EVENTS.long;

    let mut alive_spans = HashSet::new();
    let mut open_spans = vec![];
    for event in events {
        match event {
            TracingEvent::NewSpan { id, .. } => {
                assert!(alive_spans.insert(*id));
            }
            TracingEvent::SpanCloned { .. } => unreachable!(),
            TracingEvent::SpanDropped { id } => {
                assert!(!open_spans.contains(id));
                assert!(alive_spans.remove(id));
            }

            TracingEvent::SpanEntered { id } => {
                assert!(alive_spans.contains(id));
                assert!(!open_spans.contains(id));
                open_spans.push(*id);
            }
            TracingEvent::SpanExited { id } => {
                assert!(alive_spans.contains(id));
                let popped_span = open_spans.pop();
                assert_eq!(popped_span, Some(*id));
            }

            _ => { /* Do nothing */ }
        }
    }
    assert!(alive_spans.is_empty());
    assert!(open_spans.is_empty());
}

#[test]
fn call_sites_for_tracing_events() {
    let events = &EVENTS.long;

    let fields_by_span = events.iter().filter_map(|event| {
        if let TracingEvent::NewCallSite { data, .. } = event {
            if matches!(data.kind, CallSiteKind::Span) {
                let fields: Vec<_> = data.fields.iter().map(Cow::as_ref).collect();
                return Some((data.name.as_ref(), fields));
            }
        }
        None
    });
    let fields_by_span: HashMap<_, _> = fields_by_span.collect();
    assert_eq!(fields_by_span.len(), 2);
    assert_eq!(fields_by_span["fib"], ["approx"]);
    assert_eq!(fields_by_span["compute"], ["count"]);

    let mut known_metadata_ids = HashSet::new();
    let event_call_sites: Vec<_> = events
        .iter()
        .filter_map(|event| {
            if let TracingEvent::NewCallSite { id, data } = event {
                assert!(known_metadata_ids.insert(*id));
                if matches!(data.kind, CallSiteKind::Event) {
                    return Some(data);
                }
            }
            None
        })
        .collect();

    let targets: HashSet<_> = event_call_sites
        .iter()
        .map(|site| site.target.as_ref())
        .collect();
    assert_eq!(targets, HashSet::from_iter(["fib", "integration::fib"]));

    let mut call_sites_by_level = HashMap::<_, usize>::new();
    for site in &event_call_sites {
        *call_sites_by_level.entry(site.level).or_default() += 1;
    }
    assert_eq!(call_sites_by_level[&TracingLevel::Warn], 1);
    assert_eq!(call_sites_by_level[&TracingLevel::Info], 2);
    assert_eq!(call_sites_by_level[&TracingLevel::Debug], 1);
}

#[test]
fn event_fields_have_same_order() {
    let events = &EVENTS.long;

    let debug_metadata_id = events.iter().find_map(|event| {
        if let TracingEvent::NewCallSite { id, data } = event {
            if matches!(data.kind, CallSiteKind::Event) && data.level == TracingLevel::Debug {
                return Some(*id);
            }
        }
        None
    });
    let debug_metadata_id = debug_metadata_id.unwrap();

    let debug_fields = events.iter().filter_map(|event| {
        if let TracingEvent::NewEvent {
            metadata_id,
            values,
            ..
        } = event
        {
            if *metadata_id == debug_metadata_id {
                return Some(values);
            }
        }
        None
    });

    for fields in debug_fields {
        let fields: Vec<_> = fields
            .iter()
            .map(|(name, value)| (name.as_str(), value))
            .collect();
        assert_matches!(
            fields.as_slice(),
            [
                ("message", TracedValue::Object(_)),
                ("i", TracedValue::UInt(_)),
                ("current", TracedValue::UInt(_)),
            ]
        );
    }
}

fn create_fmt_subscriber() -> impl Subscriber + for<'a> LookupSpan<'a> {
    FmtSubscriber::builder()
        .pretty()
        .with_max_level(Level::TRACE)
        .with_test_writer()
        .finish()
}

/// This test are mostly about the "expected" output of `FmtSubscriber`.
/// Their output should be reviewed manually.
#[test]
fn reproducing_events_on_fmt_subscriber() {
    let events = &EVENTS.long;

    let mut consumer = TracingEventReceiver::default();
    tracing::subscriber::with_default(create_fmt_subscriber(), || {
        for event in events {
            consumer.receive(event.clone());
        }
    });
}

#[test]
fn persisting_metadata() {
    let events = &EVENTS.short;

    let mut persisted = PersistedMetadata::default();
    let mut consumer = TracingEventReceiver::new(&mut persisted, &mut PersistedSpans::default());
    tracing::subscriber::with_default(create_fmt_subscriber(), || {
        for event in events {
            consumer.receive(event.clone());
        }
    });
    consumer.persist_metadata(&mut persisted);

    let names: HashSet<_> = persisted
        .iter()
        .map(|(_, data)| data.name.as_ref())
        .collect();
    assert!(names.contains("fib"), "{names:?}");
    assert!(names.contains("compute"), "{names:?}");

    // Check that `consumer` can function after restoring `persisted` meta.
    let mut consumer = TracingEventReceiver::new(&mut persisted, &mut PersistedSpans::default());
    tracing::subscriber::with_default(create_fmt_subscriber(), || {
        for event in events {
            if !matches!(event, TracingEvent::NewCallSite { .. }) {
                consumer.receive(event.clone());
            }
        }
    });
}

#[test]
fn persisting_spans() {
    let events = &EVENTS.short;

    let mut metadata = PersistedMetadata::default();
    let mut spans = PersistedSpans::default();
    tracing::subscriber::with_default(create_fmt_subscriber(), || {
        let mut consumer = TracingEventReceiver::new(&mut metadata, &mut spans);
        for event in events {
            consumer.receive(event.clone());

            if matches!(
                event,
                TracingEvent::NewSpan { .. } | TracingEvent::SpanExited { .. }
            ) {
                // Emulate consumer reset. When the matched events are emitted,
                // spans should be non-empty.
                consumer.persist_metadata(&mut metadata);
                spans = consumer.persist_spans();
                consumer = TracingEventReceiver::new(&mut metadata, &mut spans);
            }
        }
    });
}