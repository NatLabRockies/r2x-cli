use serde::de::{Error, MapAccess, Visitor};
use serde::Deserializer;
use serde_json::{value::RawValue, Map, Value};
use std::fmt;

const MAX_JSON_DEPTH: usize = 128;

struct UniqueObject(Map<String, Value>);

struct UniqueObjectVisitor {
    depth: usize,
}

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
            let value = parse_raw(raw.get(), self.depth + 1).map_err(A::Error::custom)?;
            values.insert(key, value);
        }
        Ok(UniqueObject(values))
    }
}

// Walk raw values so that duplicate keys are rejected without rounding numbers.
fn parse_raw(raw: &str, depth: usize) -> Result<Value, serde_json::Error> {
    match raw.as_bytes().first() {
        Some(b'{') => {
            if depth > MAX_JSON_DEPTH {
                return Err(serde_json::Error::custom(format!(
                    "JSON nesting exceeds the maximum depth of {MAX_JSON_DEPTH}"
                )));
            }
            let mut deserializer = serde_json::Deserializer::from_str(raw);
            let object = deserializer.deserialize_map(UniqueObjectVisitor { depth })?;
            deserializer.end()?;
            Ok(Value::Object(object.0))
        }
        Some(b'[') => {
            if depth > MAX_JSON_DEPTH {
                return Err(serde_json::Error::custom(format!(
                    "JSON nesting exceeds the maximum depth of {MAX_JSON_DEPTH}"
                )));
            }
            let values: Vec<&RawValue> = serde_json::from_str(raw)?;
            values
                .into_iter()
                .map(|value| parse_raw(value.get(), depth + 1))
                .collect::<Result<Vec<_>, _>>()
                .map(Value::Array)
        }
        _ => serde_json::from_str(raw),
    }
}

pub(super) fn parse(contents: &str) -> Result<Value, serde_json::Error> {
    let raw: &RawValue = serde_json::from_str(contents)?;
    parse_raw(raw.get(), 1)
}

#[cfg(test)]
mod tests {
    use super::{parse, MAX_JSON_DEPTH};

    #[test]
    fn accepts_json_at_the_maximum_depth() {
        let json = format!(
            "{}0{}",
            "[".repeat(MAX_JSON_DEPTH),
            "]".repeat(MAX_JSON_DEPTH)
        );

        assert!(parse(&json).is_ok());
    }

    #[test]
    fn rejects_json_beyond_the_maximum_depth() {
        let arrays = format!(
            "{}0{}",
            "[".repeat(MAX_JSON_DEPTH + 1),
            "]".repeat(MAX_JSON_DEPTH + 1)
        );
        let objects = format!(
            "{}0{}",
            "{\"value\":".repeat(MAX_JSON_DEPTH + 1),
            "}".repeat(MAX_JSON_DEPTH + 1)
        );

        for json in [arrays, objects] {
            let error = parse(&json).expect_err("excessive nesting should fail");
            assert!(error.to_string().contains("maximum depth"));
        }
    }
}
