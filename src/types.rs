use std::collections::HashMap;

use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;

use crate::errors::{SilkwormError, SilkwormResult};

pub type Headers = HashMap<String, String>;
pub type Params = HashMap<String, String>;
pub type Meta = HashMap<String, Value>;
pub type Item = Value;

pub fn item_from<T>(value: T) -> SilkwormResult<Item>
where
    T: Serialize,
{
    serde_json::to_value(value).map_err(|err| SilkwormError::Pipeline(err.to_string()))
}

pub fn item_into<T>(item: Item) -> SilkwormResult<T>
where
    T: DeserializeOwned,
{
    serde_json::from_value(item).map_err(|err| SilkwormError::Pipeline(err.to_string()))
}

#[cfg(test)]
mod tests {
    use super::{Headers, Item, Meta, item_from, item_into};
    use serde::{Deserialize, Serialize};

    #[test]
    fn headers_and_meta_are_hashmaps() {
        let mut headers = Headers::new();
        headers.insert("accept".to_string(), "text/html".to_string());
        assert_eq!(headers.get("accept").map(String::as_str), Some("text/html"));

        let mut meta = Meta::new();
        meta.insert("count".to_string(), Item::from(2));
        assert_eq!(meta.get("count").and_then(|v| v.as_i64()), Some(2));
    }

    #[test]
    fn item_helpers_round_trip_structs() {
        #[derive(Debug, Serialize, Deserialize, PartialEq)]
        struct TestItem {
            id: u32,
            name: String,
        }

        let value = TestItem {
            id: 1,
            name: "ok".to_string(),
        };
        let item = item_from(value).expect("to item");
        let decoded: TestItem = item_into(item).expect("from item");
        assert_eq!(
            decoded,
            TestItem {
                id: 1,
                name: "ok".to_string(),
            }
        );
    }
}
