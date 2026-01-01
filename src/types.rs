use std::collections::HashMap;

use serde_json::Value;

pub type Headers = HashMap<String, String>;
pub type Params = HashMap<String, String>;
pub type Meta = HashMap<String, Value>;
pub type Item = Value;

#[cfg(test)]
mod tests {
    use super::{Headers, Item, Meta};

    #[test]
    fn headers_and_meta_are_hashmaps() {
        let mut headers = Headers::new();
        headers.insert("accept".to_string(), "text/html".to_string());
        assert_eq!(headers.get("accept").map(String::as_str), Some("text/html"));

        let mut meta = Meta::new();
        meta.insert("count".to_string(), Item::from(2));
        assert_eq!(meta.get("count").and_then(|v| v.as_i64()), Some(2));
    }
}
