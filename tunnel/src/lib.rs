//! Tunnelling tracing information across API boundary.
//!
//! This crate provides [tracing] infrastructure helpers allowing to transfer tracing events
//! across API boundary:
//!
//! - [`TracingEventSender`] is a tracing [`Subscriber`] that converts tracing events
//!   into (de)serializable presentation that can be sent elsewhere using a customizable hook.
//! - [`TracingEventReceiver`] consumes events produced by a `TracingEventSender` and relays them
//!   to the tracing infrastructure. It is assumed that the source of events may outlive
//!   both the lifetime of a particular `TracingEventReceiver` instance, and the lifetime
//!   of the program encapsulating the receiver. To deal with this, the receiver provides
//!   the means to persist / restore its state.
//!
//! Both components are used by the [Tardigrade][`tardigrade`] workflows, in case of which
//! the API boundary is the WASM client–host boundary.
//!
//! - The [`tardigrade`] client library uses [`TracingEventSender`] to send tracing events
//!   from a workflow (i.e., a WASM module instance) to the host using a WASM import function.
//! - [The Tardigrade runtime] uses [`TracingEventReceiver`] to pass traces from the workflow
//!   to the host tracing infrastructure.
//!
//! Note that
//!
//! # Crate features
//!
//! Each of the two major features outlined above is gated by the corresponding opt-in feature.
//!
//! ## `sender`
//!
//! *(Off by default)*
//!
//! Provides [`TracingEventSender`].
//!
//! ## `receiver`
//!
//! *(Off by default)*
//!
//! Provides [`TracingEventReceiver`].
//!
//! [`tardigrade`]: https://docs.rs/tardigrade
//! [tracing]: https://docs.rs/tracing/0.1/tracing
//! [`Subscriber`]: tracing_core::Subscriber
//! [The Tardigrade runtime]: https://docs.rs/tardigrade-rt

// Documentation settings.
#![cfg_attr(docsrs, feature(doc_cfg))]
#![doc(html_root_url = "https://docs.rs/tracing-tunnel/0.1.0")]
// Linter settings.
#![warn(missing_debug_implementations, missing_docs, bare_trait_objects)]
#![warn(clippy::all, clippy::pedantic)]
#![allow(clippy::must_use_candidate, clippy::module_name_repetitions)]

#[cfg(feature = "receiver")]
#[cfg_attr(docsrs, doc(cfg(feature = "receiver")))]
mod receiver;
#[cfg(feature = "sender")]
#[cfg_attr(docsrs, doc(cfg(feature = "sender")))]
mod sender;
mod serde_helpers;
mod types;

#[cfg(feature = "receiver")]
pub use crate::receiver::{PersistedMetadata, PersistedSpans, ReceiveError, TracingEventReceiver};
#[cfg(feature = "sender")]
pub use crate::sender::TracingEventSender;
pub use crate::types::{
    CallSiteData, CallSiteKind, DebugObject, MetadataId, RawSpanId, TracedError, TracedValue,
    TracingEvent, TracingLevel, ValueVisitor,
};
