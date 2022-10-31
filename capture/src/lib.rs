//! Capturing tracing spans and events, e.g. for testing purposes.
//!
//! The core type in this crate is [`CaptureLayer`], a tracing [`Layer`] that can be used
//! to capture tracing spans and events.
//!
//! # Examples
//!
//! ```
//! use tracing::Level;
//! use tracing_subscriber::layer::SubscriberExt;
//! use tracing_capture::{CaptureLayer, SharedStorage};
//!
//! let subscriber = tracing_subscriber::fmt()
//!     .pretty()
//!     .with_max_level(Level::INFO)
//!     .finish();
//! // Add the capturing layer.
//! let storage = SharedStorage::default();
//! let subscriber = subscriber.with(CaptureLayer::new(&storage));
//!
//! // Capture tracing information.
//! tracing::subscriber::with_default(subscriber, || {
//!     tracing::info_span!("test", num = 42_i64).in_scope(|| {
//!         tracing::warn!("I feel disturbance in the Force...");
//!     });
//! });
//!
//! // Inspect the only captured span.
//! let storage = storage.lock();
//! assert_eq!(storage.all_spans().len(), 1);
//! let span = storage.all_spans().next().unwrap();
//! assert_eq!(span["num"], 42_i64);
//! assert_eq!(span.stats().entered, 1);
//! assert!(span.stats().is_closed);
//!
//! // Inspect the only event in the span.
//! let event = span.events().next().unwrap();
//! assert_eq!(*event.metadata().level(), Level::WARN);
//! assert_eq!(
//!     event["message"].as_debug_str(),
//!     Some("I feel disturbance in the Force...")
//! );
//! ```
//!
//! # Alternatives / similar tools
//!
//! - [`tracing-test`] is a lower-level alternative.
//! - [`tracing-fluent-assertions`] is more similar in its goals, but differs significantly
//!   in the API design; e.g., the assertions need to be declared before the capture.
//!
//! [`tracing-test`]: https://docs.rs/tracing-test
//! [`tracing-fluent-assertions`]: https://docs.rs/tracing-fluent-assertions

// Documentation settings.
#![doc(html_root_url = "https://docs.rs/tracing-capture/0.1.0")]
// Linter settings.
#![warn(missing_debug_implementations, missing_docs, bare_trait_objects)]
#![warn(clippy::all, clippy::pedantic)]
#![allow(clippy::must_use_candidate, clippy::module_name_repetitions)]

use tracing_core::Metadata;

use std::{fmt, ops};

mod iter;
mod layer;
pub mod predicates;

mod sealed {
    pub trait Sealed {}
}

pub use crate::{
    iter::{CapturedEvents, CapturedSpans},
    layer::{CaptureLayer, SharedStorage, Storage},
};

use tracing_tunnel::{TracedValue, TracedValues};

/// Marker trait for captured objects (spans and events).
pub trait Captured: fmt::Debug + sealed::Sealed {}

#[derive(Debug)]
struct CapturedEventInner {
    metadata: &'static Metadata<'static>,
    values: TracedValues<&'static str>,
    parent_id: Option<CapturedSpanId>,
}

type CapturedEventId = id_arena::Id<CapturedEventInner>;

/// Captured tracing event containing a reference to its [`Metadata`] and values that the event
/// was created with.
#[derive(Debug, Clone, Copy)]
pub struct CapturedEvent<'a> {
    inner: &'a CapturedEventInner,
    storage: &'a Storage,
}

impl<'a> CapturedEvent<'a> {
    /// Provides a reference to the event metadata.
    pub fn metadata(&self) -> &'static Metadata<'static> {
        self.inner.metadata
    }

    /// Iterates over values associated with the event.
    pub fn values(&self) -> impl Iterator<Item = (&'static str, &TracedValue)> + '_ {
        self.inner.values.iter().map(|(name, value)| (*name, value))
    }

    /// Returns a value for the specified field, or `None` if the value is not defined.
    pub fn value(&self, name: &str) -> Option<&TracedValue> {
        self.inner
            .values
            .iter()
            .find_map(|(s, value)| if *s == name { Some(value) } else { None })
    }

    /// Returns the parent span for this event, or `None` if is not tied to a captured span.
    pub fn parent(&self) -> Option<CapturedSpan<'a>> {
        self.inner.parent_id.map(|id| self.storage.span(id))
    }

    /// Returns the references to the ancestor spans, starting from the direct parent
    /// and ending in one of [root spans](Storage::root_spans()).
    pub fn ancestors(&self) -> impl Iterator<Item = CapturedSpan<'a>> + '_ {
        std::iter::successors(self.parent(), CapturedSpan::parent)
    }
}

impl ops::Index<&str> for CapturedEvent<'_> {
    type Output = TracedValue;

    fn index(&self, index: &str) -> &Self::Output {
        self.value(index)
            .unwrap_or_else(|| panic!("field `{index}` is not contained in event"))
    }
}

impl sealed::Sealed for CapturedEvent<'_> {}
impl Captured for CapturedEvent<'_> {}

/// Statistics about a [`CapturedSpan`].
#[derive(Debug, Clone, Copy, Default)]
#[non_exhaustive]
pub struct SpanStats {
    /// Number of times the span was entered.
    pub entered: usize,
    /// Number of times the span was exited.
    pub exited: usize,
    /// Is the span closed (dropped)?
    pub is_closed: bool,
}

#[derive(Debug)]
struct CapturedSpanInner {
    metadata: &'static Metadata<'static>,
    values: TracedValues<&'static str>,
    stats: SpanStats,
    parent_id: Option<CapturedSpanId>,
    child_ids: Vec<CapturedSpanId>,
    event_ids: Vec<CapturedEventId>,
}

type CapturedSpanId = id_arena::Id<CapturedSpanInner>;

/// Captured tracing span containing a reference to its [`Metadata`], values that the span
/// was created with, [stats](SpanStats), and descendant [`CapturedEvent`]s.
#[derive(Debug, Clone, Copy)]
pub struct CapturedSpan<'a> {
    inner: &'a CapturedSpanInner,
    storage: &'a Storage,
}

impl<'a> CapturedSpan<'a> {
    /// Provides a reference to the span metadata.
    pub fn metadata(&self) -> &'static Metadata<'static> {
        self.inner.metadata
    }

    /// Iterates over values that the span was created with, or which were recorded later.
    pub fn values(&self) -> impl Iterator<Item = (&'static str, &TracedValue)> + '_ {
        self.inner.values.iter().map(|(name, value)| (*name, value))
    }

    /// Returns a value for the specified field, or `None` if the value is not defined.
    pub fn value(&self, name: &str) -> Option<&TracedValue> {
        self.inner
            .values
            .iter()
            .find_map(|(s, value)| if *s == name { Some(value) } else { None })
    }

    /// Returns statistics about span operations.
    pub fn stats(&self) -> SpanStats {
        self.inner.stats
    }

    /// Returns events attached to this span.
    pub fn events(&self) -> CapturedEvents<'a> {
        CapturedEvents::from_slice(self.storage, &self.inner.event_ids)
    }

    /// Returns the reference to the parent span, if any.
    pub fn parent(&self) -> Option<Self> {
        self.inner.parent_id.map(|id| self.storage.span(id))
    }

    /// Returns the references to the ancestor spans, starting from the direct parent
    /// and ending in one of [root spans](Storage::root_spans()).
    pub fn ancestors(&self) -> impl Iterator<Item = CapturedSpan<'a>> + '_ {
        std::iter::successors(self.parent(), Self::parent)
    }

    /// Iterates over direct children of this span, in the order of their capture.
    pub fn children(&self) -> CapturedSpans<'a> {
        CapturedSpans::from_slice(self.storage, &self.inner.child_ids)
    }
}

impl ops::Index<&str> for CapturedSpan<'_> {
    type Output = TracedValue;

    fn index(&self, index: &str) -> &Self::Output {
        self.value(index)
            .unwrap_or_else(|| panic!("field `{index}` is not contained in span"))
    }
}

impl sealed::Sealed for CapturedSpan<'_> {}
impl Captured for CapturedSpan<'_> {}

#[cfg(doctest)]
doc_comment::doctest!("../README.md");
