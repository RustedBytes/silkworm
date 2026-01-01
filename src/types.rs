use std::collections::HashMap;

use serde_json::Value;

pub type Headers = HashMap<String, String>;
pub type Params = HashMap<String, String>;
pub type Meta = HashMap<String, Value>;
pub type Item = Value;
