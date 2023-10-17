use std::error::Error;

use crate::error::{ConfigError, Unexpected};
use crate::map::Map;
use crate::value::{Value, ValueKind};
use serde::Deserialize;
use serde_with::rust::deserialize_ignore_any;

/// Describes a format of configuration source data
///
/// Implementations of this trait can be used to convert [`File`](crate::File) sources to configuration data.
///
/// There can be various formats, some of them provided by this library, such as JSON, Yaml and other.
/// This trait enables users of the library to easily define their own, even proprietary formats without
/// the need to alter library sources.
///
/// What is more, it is recommended to use this trait with custom [`Source`](crate::Source)s and their async counterparts.
pub trait Format {
    /// Parses provided content into configuration values understood by the library.
    ///
    /// It also allows specifying optional URI of the source associated with format instance that can facilitate debugging.
    fn parse(
        &self,
        uri: Option<&String>,
        text: &str,
    ) -> Result<Map<String, Value>, Box<dyn Error + Send + Sync>>;
}

// Have a proper error fire if the root of a file is ever not a Table
pub fn extract_root_table(
    uri: Option<&String>,
    value: Value,
) -> Result<Map<String, Value>, Box<dyn Error + Send + Sync>> {
    match value.kind {
        ValueKind::Table(map) => Ok(map),
        ValueKind::Nil => Err(Unexpected::Unit),
        ValueKind::Array(_value) => Err(Unexpected::Seq),
        ValueKind::Boolean(value) => Err(Unexpected::Bool(value)),
        ValueKind::I64(value) => Err(Unexpected::I64(value)),
        ValueKind::I128(value) => Err(Unexpected::I128(value)),
        ValueKind::U64(value) => Err(Unexpected::U64(value)),
        ValueKind::U128(value) => Err(Unexpected::U128(value)),
        ValueKind::Float(value) => Err(Unexpected::Float(value)),
        ValueKind::String(value) => Err(Unexpected::Str(value)),
    }
    .map_err(|err| ConfigError::invalid_root(uri, err))
    .map_err(|err| Box::new(err) as Box<dyn Error + Send + Sync>)
}

// Equivalent to ValueKind, except Table + Array store the same enum
// Useful for serde to serialize values into, then convert to Value.
// NOTE: Order of variants is important. Serde will use whichever
// the input successfully deserializes into first.
#[derive(serde::Deserialize, Debug)]
#[serde(untagged)]
pub enum ParsedValue {
    Boolean(bool),
    I64(i64),
    I128(i128),
    U64(u64),
    U128(u128),
    Float(f64),
    #[serde(deserialize_with = "deserialize_parsed_string")]
    String(String),
    #[serde(deserialize_with = "deserialize_parsed_map")]
    Table(Map<String, Self>),
    Array(Vec<Self>),
    Option(Option<Box<Self>>),
    // If nothing else above matched, use Nil:
    #[serde(deserialize_with = "deserialize_ignore_any")]
    Nil,
}

// Value wrap ValueKind values, with optional uri (origin)
pub fn from_parsed_value(uri: Option<&String>, value: ParsedValue) -> Value {
    let vk = match value {
        ParsedValue::Nil => ValueKind::Nil,
        ParsedValue::String(v) => ValueKind::String(v),
        ParsedValue::I64(v) => ValueKind::I64(v),
        ParsedValue::I128(v) => ValueKind::I128(v),
        ParsedValue::U64(v) => ValueKind::U64(v),
        ParsedValue::U128(v) => ValueKind::U128(v),
        ParsedValue::Float(v) => ValueKind::Float(v),
        ParsedValue::Boolean(v) => ValueKind::Boolean(v),

        ParsedValue::Table(table) => {
            let m = table
                .into_iter()
                .map(|(k, v)| (k, from_parsed_value(uri, v)))
                .collect();

            ValueKind::Table(m)
        }

        ParsedValue::Array(array) => {
            let l = array
                .into_iter()
                .map(|v| from_parsed_value(uri, v))
                .collect();

            ValueKind::Array(l)
        }

        // Boxed value must be dereferenced:
        ParsedValue::Option(v) => match v {
            Some(boxed) => from_parsed_value(uri, *boxed).kind,
            None => ValueKind::Nil,
        },
    };

    Value::new(uri, vk)
}

fn deserialize_parsed_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::de::Deserializer<'de>,
{
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum ParsedString {
        // Anything that can deserialize into a string successfully:
        String(String),
        // Config specific support for types that need string conversion:
        Char(char),
        #[cfg(feature = "toml")]
        TomlDateTime(toml::value::Datetime),
    }

    match ParsedString::deserialize(deserializer)? {
        ParsedString::String(v) => Ok(v),
        ParsedString::Char(v) => Ok(v.to_string()),
        #[cfg(feature = "toml")]
        ParsedString::TomlDateTime(v) => Ok(v.to_string()),
    }
}

fn deserialize_parsed_map<'de, D>(deserializer: D) -> Result<Map<String, ParsedValue>, D::Error>
where
    D: serde::de::Deserializer<'de>,
{
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum ParsedMap {
        // Anything that can deserialize into a Map successfully:
        Table(Map<String, ParsedValue>),
        // Config specific support for types that need string conversion:
        #[cfg(feature = "yaml")]
        YamlMap(serde_yaml::Mapping),
    }

    match ParsedMap::deserialize(deserializer)? {
        ParsedMap::Table(v) => Ok(v),
        #[cfg(feature = "yaml")]
        ParsedMap::YamlMap(table) => {
            table
                .into_iter()
                .map(|(key, value)| {
                    let key = match key {
                        serde_yaml::Value::Number(k) => Some(k.to_string()),
                        serde_yaml::Value::String(k) => Some(k),
                        _ => None,
                    };
                    let value = serde_yaml::from_value::<ParsedValue>(value).ok();

                    // Option to Result:
                    match (key, value) {
                        (Some(k), Some(v)) => Ok((k, v)),
                        _ => Err(serde::de::Error::custom("should not be serialized to Map")),
                    }
                })
                .collect()
        }
    }
}
