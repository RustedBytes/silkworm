# Errors and Shared Types

Silkworm uses a small set of error variants and shared type aliases to keep
spider code lightweight and ergonomic.

## Errors

`SilkwormError` provides error categories for HTTP, selectors, spider logic,
pipeline handling, configuration, and IO.

Code:
- Error types: `../src/errors.rs`

```rust
pub enum SilkwormError {
    Http(String),
    Selector(String),
    Spider(String),
    Pipeline(String),
    Config(String),
    Io(std::io::Error),
}
```

## Shared Types

Type aliases simplify signatures throughout the API:

- `Headers`: `HashMap<String, String>`
- `Params`: `HashMap<String, String>`
- `Meta`: `HashMap<String, serde_json::Value>`
- `Item`: `serde_json::Value`

Conversion helpers:
- `item_from`: serialize a typed struct into `Item`.
- `item_into`: deserialize an `Item` back into a typed struct.

Code:
- Type aliases and helpers: `../src/types.rs`

```rust
use serde::{Deserialize, Serialize};
use silkworm::{item_from, item_into, Item};

#[derive(Serialize, Deserialize)]
struct User {
    id: u64,
    name: String,
}

let item: Item = item_from(User { id: 1, name: "Ada".to_string() })?;
let user: User = item_into(item)?;
```
