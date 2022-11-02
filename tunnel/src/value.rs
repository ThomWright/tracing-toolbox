//! `TracedValue` and closely related types.

use serde::{Deserialize, Serialize};

use std::{borrow::Borrow, error, fmt};

/// (De)serializable presentation for an error recorded as a value in a tracing span or event.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[non_exhaustive]
pub struct TracedError {
    /// Error message produced by its [`Display`](fmt::Display) implementation.
    pub message: String,
    /// Error [source](error::Error::source()).
    pub source: Option<Box<TracedError>>,
}

impl TracedError {
    fn new(err: &(dyn error::Error + 'static)) -> Self {
        Self {
            message: err.to_string(),
            source: err.source().map(|source| Box::new(Self::new(source))),
        }
    }
}

impl fmt::Display for TracedError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl error::Error for TracedError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        self.source
            .as_ref()
            .map(|source| source.as_ref() as &(dyn error::Error + 'static))
    }
}

/// Opaque wrapper for a [`Debug`](fmt::Debug)gable object recorded as a value
/// in a tracing span or event.
#[derive(Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DebugObject(String);

impl fmt::Debug for DebugObject {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// Returns the [`Debug`](fmt::Debug) representation of the object.
impl AsRef<str> for DebugObject {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

/// Value recorded in a tracing span or event.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum TracedValue {
    /// Boolean value.
    Bool(bool),
    /// Signed integer value.
    Int(i128),
    /// Unsigned integer value.
    UInt(u128),
    /// Floating-point value.
    Float(f64),
    /// String value.
    String(String),
    /// Opaque object implementing the [`Debug`](fmt::Debug) trait.
    Object(DebugObject),
    /// Opaque error.
    Error(TracedError),
}

impl TracedValue {
    #[doc(hidden)] // public for testing purposes
    pub fn debug(object: &dyn fmt::Debug) -> Self {
        Self::Object(DebugObject(format!("{object:?}")))
    }

    /// Tries to convert this value into a specific subtype. Returns `None` if the conversion
    /// fails.
    pub fn try_as<'s, T>(&'s self) -> Option<T::Output>
    where
        T: FromTracedValue<'s> + ?Sized,
    {
        T::from_value(self)
    }

    /// Checks whether this value is a [`DebugObject`] with the same [`Debug`](fmt::Debug)
    /// output as the provided `object`.
    pub fn is_debug(&self, object: &dyn fmt::Debug) -> bool {
        match self {
            Self::Object(value) => value.0 == format!("{object:?}"),
            _ => false,
        }
    }

    /// Returns value as a [`Debug`](fmt::Debug) string output, or `None` if this value
    /// is not [`Self::Object`].
    pub fn as_debug_str(&self) -> Option<&str> {
        match self {
            Self::Object(value) => Some(&value.0),
            _ => None,
        }
    }

    pub(crate) fn error(err: &(dyn error::Error + 'static)) -> Self {
        Self::Error(TracedError::new(err))
    }
}

/// Fallible conversion from a [`TracedValue`] reference.
pub trait FromTracedValue<'a> {
    /// Output of the conversion.
    type Output: Borrow<Self> + 'a;
    /// Performs the conversion.
    fn from_value(value: &'a TracedValue) -> Option<Self::Output>;
}

impl<'a> FromTracedValue<'a> for str {
    type Output = &'a str;

    fn from_value(value: &'a TracedValue) -> Option<Self::Output> {
        match value {
            TracedValue::String(value) => Some(value),
            _ => None,
        }
    }
}

macro_rules! impl_value_conversions {
    (TracedValue :: $variant:ident ($source:ty)) => {
        impl From<$source> for TracedValue {
            fn from(value: $source) -> Self {
                Self::$variant(value)
            }
        }

        impl PartialEq<$source> for TracedValue {
            fn eq(&self, other: &$source) -> bool {
                match self {
                    Self::$variant(value) => value == other,
                    _ => false,
                }
            }
        }

        impl PartialEq<TracedValue> for $source {
            fn eq(&self, other: &TracedValue) -> bool {
                other == self
            }
        }

        impl FromTracedValue<'_> for $source {
            type Output = Self;

            fn from_value(value: &TracedValue) -> Option<Self::Output> {
                match value {
                    TracedValue::$variant(value) => Some(*value),
                    _ => None,
                }
            }
        }
    };

    (TracedValue :: $variant:ident ($source:ty as $field_ty:ty)) => {
        impl From<$source> for TracedValue {
            fn from(value: $source) -> Self {
                Self::$variant(value.into())
            }
        }

        impl PartialEq<$source> for TracedValue {
            fn eq(&self, other: &$source) -> bool {
                match self {
                    Self::$variant(value) => *value == <$field_ty>::from(*other),
                    _ => false,
                }
            }
        }

        impl PartialEq<TracedValue> for $source {
            fn eq(&self, other: &TracedValue) -> bool {
                other == self
            }
        }

        impl FromTracedValue<'_> for $source {
            type Output = Self;

            fn from_value(value: &TracedValue) -> Option<Self::Output> {
                match value {
                    TracedValue::$variant(value) => (*value).try_into().ok(),
                    _ => None,
                }
            }
        }
    };
}

impl_value_conversions!(TracedValue::Bool(bool));
impl_value_conversions!(TracedValue::Int(i128));
impl_value_conversions!(TracedValue::Int(i64 as i128));
impl_value_conversions!(TracedValue::UInt(u128));
impl_value_conversions!(TracedValue::UInt(u64 as u128));
impl_value_conversions!(TracedValue::Float(f64));

impl PartialEq<str> for TracedValue {
    fn eq(&self, other: &str) -> bool {
        match self {
            Self::String(value) => value == other,
            _ => false,
        }
    }
}

impl PartialEq<TracedValue> for str {
    fn eq(&self, other: &TracedValue) -> bool {
        other == self
    }
}

impl From<&str> for TracedValue {
    fn from(value: &str) -> Self {
        Self::String(value.to_owned())
    }
}

impl PartialEq<&str> for TracedValue {
    fn eq(&self, other: &&str) -> bool {
        match self {
            Self::String(value) => value == *other,
            _ => false,
        }
    }
}

impl PartialEq<TracedValue> for &str {
    fn eq(&self, other: &TracedValue) -> bool {
        other == self
    }
}
