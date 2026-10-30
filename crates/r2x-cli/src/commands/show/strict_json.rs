use serde::de::{Error, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::{value::RawValue, Map, Value};
use std::fmt;

struct UniqueObject(Map<String, Value>);

impl<'de> Deserialize<'de> for UniqueObject {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_map(UniqueObjectVisitor)
    }
}

struct UniqueObjectVisitor;

impl<'de> Visitor<'de> for UniqueObjectVisitor {
    type Value = UniqueObject;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON object with unique keys")
    }

    fn visit_map<A: MapAccess<'de>>(self, mut object: A) -> Result<UniqueObject, A::Error> {
        let mut values = Map::new();
        while let Some(key) = object.next_key::<String>()? {
            if values.contains_key(&key) {
                return Err(A::Error::custom(format!("Duplicate JSON key '{key}'")));
            }
            let raw: &RawValue = object.next_value()?;
            let value = parse_raw(raw.get()).map_err(A::Error::custom)?;
            values.insert(key, value);
        }
        Ok(UniqueObject(values))
    }
}

// Walk raw values so that duplicate keys are rejected without rounding numbers.
fn parse_raw(raw: &str) -> Result<Value, serde_json::Error> {
    match raw.as_bytes().first() {
        Some(b'{') => {
            serde_json::from_str::<UniqueObject>(raw).map(|object| Value::Object(object.0))
        }
        Some(b'[') => {
            let values: Vec<&RawValue> = serde_json::from_str(raw)?;
            values
                .into_iter()
                .map(|value| parse_raw(value.get()))
                .collect::<Result<Vec<_>, _>>()
                .map(Value::Array)
        }
        _ => serde_json::from_str(raw),
    }
}

pub(super) fn parse(contents: &str) -> Result<Value, serde_json::Error> {
    let raw: &RawValue = serde_json::from_str(contents)?;
    parse_raw(raw.get())
}
