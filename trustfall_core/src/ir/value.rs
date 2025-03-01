/// IR of the values of Trustfall fields.
use async_graphql_value::{ConstValue, Number, Value};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Values of fields in Trustfall.
///
/// For version that is serialized as an untagged enum, see [TransparentValue].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum FieldValue {
    // Order may matter here! Deserialization, if ever configured for untagged serialization,
    // will attempt each variant in order until the first one that matches. Int64 must be
    // above Uint64, which must be above Float64.
    // This is because we want to prioritize the standard Integer GraphQL type over our custom u64,
    // and prioritize exact integers over lossy floats.
    Null,
    /// AKA integer
    Int64(i64),
    Uint64(u64),
    /// AKA Float, and also not allowed to be NaN
    Float64(f64),
    String(String),
    Boolean(bool),
    DateTimeUtc(DateTime<Utc>),
    Enum(String),
    List(Vec<FieldValue>),
}

/// Values of fields in GraphQL types.
///
/// Same as [FieldValue], but serialized as an untagged enum,
/// which may be more suitable e.g. when serializing to JSON.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TransparentValue {
    // Order may matter here! Deserialization, if ever configured for untagged serialization,
    // will attempt each variant in order until the first one that matches. Int64 must be
    // above Uint64, which must be above Float64.
    // This is because we want to prioritize the standard Integer GraphQL type over our custom u64,
    // and prioritize exact integers over lossy floats.
    Null,
    Int64(i64), // AKA Integer
    Uint64(u64),
    Float64(f64), // AKA Float, and also not allowed to be NaN
    String(String),
    Boolean(bool),
    DateTimeUtc(DateTime<Utc>),
    Enum(String),
    List(Vec<TransparentValue>),
}

impl From<FieldValue> for TransparentValue {
    fn from(value: FieldValue) -> Self {
        match value {
            FieldValue::Null => TransparentValue::Null,
            FieldValue::Int64(x) => TransparentValue::Int64(x),
            FieldValue::Uint64(x) => TransparentValue::Uint64(x),
            FieldValue::Float64(x) => TransparentValue::Float64(x),
            FieldValue::String(x) => TransparentValue::String(x),
            FieldValue::Boolean(x) => TransparentValue::Boolean(x),
            FieldValue::DateTimeUtc(x) => TransparentValue::DateTimeUtc(x),
            FieldValue::Enum(x) => TransparentValue::Enum(x),
            FieldValue::List(x) => {
                TransparentValue::List(x.into_iter().map(|v| v.into()).collect())
            }
        }
    }
}

impl From<TransparentValue> for FieldValue {
    fn from(value: TransparentValue) -> Self {
        match value {
            TransparentValue::Null => FieldValue::Null,
            TransparentValue::Int64(x) => FieldValue::Int64(x),
            TransparentValue::Uint64(x) => FieldValue::Uint64(x),
            TransparentValue::Float64(x) => FieldValue::Float64(x),
            TransparentValue::String(x) => FieldValue::String(x),
            TransparentValue::Boolean(x) => FieldValue::Boolean(x),
            TransparentValue::DateTimeUtc(x) => FieldValue::DateTimeUtc(x),
            TransparentValue::Enum(x) => FieldValue::Enum(x),
            TransparentValue::List(x) => {
                FieldValue::List(x.into_iter().map(|v| v.into()).collect())
            }
        }
    }
}

impl FieldValue {
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            FieldValue::Uint64(u) => (*u).try_into().ok(),
            FieldValue::Int64(i) => Some(*i),
            FieldValue::Null
            | FieldValue::Float64(_)
            | FieldValue::String(_)
            | FieldValue::Boolean(_)
            | FieldValue::DateTimeUtc(_)
            | FieldValue::List(_)
            | FieldValue::Enum(_) => None,
        }
    }

    pub fn as_u64(&self) -> Option<u64> {
        match self {
            FieldValue::Uint64(u) => Some(*u),
            FieldValue::Int64(i) => (*i).try_into().ok(),
            FieldValue::Null
            | FieldValue::Float64(_)
            | FieldValue::String(_)
            | FieldValue::Boolean(_)
            | FieldValue::DateTimeUtc(_)
            | FieldValue::List(_)
            | FieldValue::Enum(_) => None,
        }
    }

    pub fn as_usize(&self) -> Option<usize> {
        match self {
            FieldValue::Uint64(u) => (*u).try_into().ok(),
            FieldValue::Int64(i) => (*i).try_into().ok(),
            FieldValue::Null
            | FieldValue::Float64(_)
            | FieldValue::String(_)
            | FieldValue::Boolean(_)
            | FieldValue::DateTimeUtc(_)
            | FieldValue::List(_)
            | FieldValue::Enum(_) => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            FieldValue::String(s) => Some(s.as_str()),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            FieldValue::Boolean(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_vec<'a, T>(&'a self, inner: impl Fn(&'a FieldValue) -> Option<T>) -> Option<Vec<T>> {
        match self {
            FieldValue::List(l) => {
                let maybe_vec: Option<Vec<T>> = l.iter().map(inner).collect();
                maybe_vec
            }
            _ => None,
        }
    }
}

impl PartialEq for FieldValue {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Uint64(l0), Self::Uint64(r0)) => l0 == r0,
            (Self::Int64(l0), Self::Int64(r0)) => l0 == r0,
            (Self::Float64(l0), Self::Float64(r0)) => {
                assert!(l0.is_finite());
                assert!(r0.is_finite());
                l0 == r0
            }
            (Self::String(l0), Self::String(r0)) => l0 == r0,
            (Self::Boolean(l0), Self::Boolean(r0)) => l0 == r0,
            (Self::DateTimeUtc(l0), Self::DateTimeUtc(r0)) => l0 == r0,
            (Self::List(l0), Self::List(r0)) => l0 == r0,
            (Self::Enum(l0), Self::Enum(r0)) => l0 == r0,
            _ => core::mem::discriminant(self) == core::mem::discriminant(other),
        }
    }
}

impl Eq for FieldValue {}

impl AsRef<FieldValue> for FieldValue {
    fn as_ref(&self) -> &FieldValue {
        self
    }
}

impl From<String> for FieldValue {
    fn from(v: String) -> Self {
        Self::String(v)
    }
}

impl From<&String> for FieldValue {
    fn from(v: &String) -> Self {
        Self::String(v.clone())
    }
}

impl From<&str> for FieldValue {
    fn from(v: &str) -> Self {
        Self::from(v.to_owned())
    }
}

impl From<bool> for FieldValue {
    fn from(v: bool) -> Self {
        Self::Boolean(v)
    }
}

/// Represents a finite (non-infinite, not-NaN) [f64] value
pub struct FiniteF64(f64);
impl From<FiniteF64> for FieldValue {
    fn from(f: FiniteF64) -> FieldValue {
        FieldValue::Float64(f.0)
    }
}

macro_rules! impl_finite_f64_try_from_float {
    ( $( $Float: ident )+ ) => {
        $(
            impl TryFrom<$Float> for FiniteF64 {
                type Error = ($Float, &'static str);

                fn try_from(v: $Float) -> Result<Self, Self::Error> {
                    if v.is_finite() {
                        Ok(Self(v.into()))
                    } else {
                        Err((v, "not a finite (non-infinite, not-NaN) value"))
                    }
                }
            }
        )+
    }
}

impl_finite_f64_try_from_float!(f32 f64);

macro_rules! impl_field_value_from_int {
    ( $( $Int: ident )+ ) => {
        $(
            impl From<$Int> for FieldValue {
                fn from(v: $Int) -> Self {
                    Self::Int64(v.into())
                }
            }
        )+
    }
}

macro_rules! impl_field_value_from_uint {
    ( $( $Uint: ident )+ ) => {
        $(
            impl From<$Uint> for FieldValue {
                fn from(v: $Uint) -> Self {
                    Self::Uint64(v.into())
                }
            }
        )+
    }
}

impl_field_value_from_int!(i8 i16 i32 i64);
impl_field_value_from_uint!(u8 u16 u32 u64);

impl From<DateTime<Utc>> for FieldValue {
    fn from(v: DateTime<Utc>) -> Self {
        Self::DateTimeUtc(v)
    }
}

impl TryFrom<Option<f32>> for FieldValue {
    type Error = (f32, &'static str);

    fn try_from(value: Option<f32>) -> Result<Self, Self::Error> {
        match value {
            None => Ok(FieldValue::Null),
            Some(v) => {
                let finite_f64 = FiniteF64::try_from(v);
                finite_f64.map(|x| x.into())
            }
        }
    }
}

impl TryFrom<Option<f64>> for FieldValue {
    type Error = (f64, &'static str);

    fn try_from(value: Option<f64>) -> Result<Self, Self::Error> {
        match value {
            None => Ok(FieldValue::Null),
            Some(v) => Ok(FiniteF64::try_from(v)?.into()),
        }
    }
}

impl<T: Into<FieldValue>> From<Option<T>> for FieldValue {
    fn from(opt: Option<T>) -> FieldValue {
        match opt {
            Some(inner) => inner.into(),
            None => FieldValue::Null,
        }
    }
}

impl<T: Into<FieldValue>> FromIterator<T> for FieldValue {
    fn from_iter<I>(iter: I) -> Self
    where
        I: IntoIterator<Item = T>,
    {
        FieldValue::List(iter.into_iter().map(Into::into).collect())
    }
}

impl<T: Into<FieldValue>> From<Vec<T>> for FieldValue {
    fn from(vec: Vec<T>) -> FieldValue {
        vec.into_iter().collect()
    }
}

/// Converts a JSON number to a [FieldValue]
fn convert_number_to_field_value(n: &Number) -> Result<FieldValue, String> {
    // The order here matters!
    // Int64 must be before Uint64, which must be before Float64.
    // See the comment near the definition of FieldValue for details.
    if let Some(i) = n.as_i64() {
        Ok(FieldValue::Int64(i))
    } else if let Some(u) = n.as_u64() {
        Ok(FieldValue::Uint64(u))
    } else if let Some(f) = n.as_f64() {
        Ok(FieldValue::Float64(f))
    } else {
        unreachable!()
    }
}

impl TryFrom<Value> for FieldValue {
    type Error = String;

    fn try_from(value: Value) -> Result<Self, Self::Error> {
        match value {
            Value::Null => Ok(Self::Null),
            Value::Number(n) => convert_number_to_field_value(&n),
            Value::String(s) => Ok(Self::String(s)),
            Value::Boolean(b) => Ok(Self::Boolean(b)),
            Value::List(l) => l
                .into_iter()
                .map(Self::try_from)
                .collect::<Result<Self, _>>(),
            Value::Enum(n) => {
                // We have an enum value, so we know the variant name but the variant on its own
                // doesn't tell us the name of the enum type it belongs in. We'll have to determine
                // the name of the enum type from context. For now, it's None.
                Ok(Self::Enum(n.to_string()))
            }
            Value::Binary(_) => Err(String::from("Binary values are not supported")),
            Value::Variable(_) => Err(String::from("Cannot use a variable reference")),
            Value::Object(_) => Err(String::from("Object values are not supported")),
        }
    }
}

impl TryFrom<ConstValue> for FieldValue {
    type Error = String;

    fn try_from(value: ConstValue) -> Result<Self, Self::Error> {
        value.into_value().try_into()
    }
}

#[cfg(test)]
mod tests {
    use super::{FieldValue, FiniteF64};

    #[test]
    fn test_field_value_into() {
        let test_data: Vec<(FieldValue, FieldValue)> = vec![
            (123i64.into(), FieldValue::Int64(123)),
            (123u64.into(), FieldValue::Uint64(123)),
            (Option::<i64>::Some(123i64).into(), FieldValue::Int64(123)),
            (Option::<u64>::Some(123u64).into(), FieldValue::Uint64(123)),
            (
                FiniteF64::try_from(3.15).unwrap().into(),
                FieldValue::Float64(3.15),
            ),
            (false.into(), FieldValue::Boolean(false)),
            ("a &str".into(), FieldValue::String("a &str".to_string())),
            (
                "a String".to_string().into(),
                FieldValue::String("a String".to_string()),
            ),
            (
                (&"a &String".to_string()).into(),
                FieldValue::String("a &String".to_string()),
            ),
            (Option::<i64>::None.into(), FieldValue::Null),
            (
                vec![1, 2].into(),
                FieldValue::List(vec![FieldValue::Int64(1), FieldValue::Int64(2)]),
            ),
        ];

        for (actual_value, expected_value) in test_data {
            assert_eq!(actual_value, expected_value);
        }
    }
}
