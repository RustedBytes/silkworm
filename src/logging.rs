use fasttime::DateTime;
use std::env;
use std::io::{self, Write};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Level {
    Debug = 1,
    Info = 2,
    Warn = 3,
    Error = 4,
}

static MIN_LEVEL: OnceLock<Level> = OnceLock::new();

fn min_level() -> Level {
    *MIN_LEVEL.get_or_init(|| {
        let level = env::var("SILKWORM_LOG_LEVEL").unwrap_or_else(|_| "INFO".to_string());
        match level.to_uppercase().as_str() {
            "TRACE" | "DEBUG" => Level::Debug,
            "INFO" => Level::Info,
            "WARN" | "WARNING" => Level::Warn,
            "ERROR" | "ERR" | "FAIL" | "FATAL" | "CRITICAL" => Level::Error,
            _ => Level::Info,
        }
    })
}

#[derive(Clone, Debug)]
pub struct Logger {
    context: Vec<(String, String)>,
}

impl Logger {
    pub fn bind(&self, key: &str, value: impl ToString) -> Logger {
        let mut context = self.context.clone();
        context.push((key.to_string(), value.to_string()));
        Logger { context }
    }

    pub fn debug(&self, message: &str, fields: &[(&str, String)]) {
        self.log(Level::Debug, message, fields);
    }

    pub fn info(&self, message: &str, fields: &[(&str, String)]) {
        self.log(Level::Info, message, fields);
    }

    pub fn warn(&self, message: &str, fields: &[(&str, String)]) {
        self.log(Level::Warn, message, fields);
    }

    pub fn error(&self, message: &str, fields: &[(&str, String)]) {
        self.log(Level::Error, message, fields);
    }

    fn log(&self, level: Level, message: &str, fields: &[(&str, String)]) {
        let min_level = min_level();
        if level < min_level {
            return;
        }

        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| {
                DateTime::from_unix_timestamp(d.as_secs() as i64, d.subsec_nanos() as i32)
                    .map(|dt| dt.to_string())
                    .unwrap_or_else(|_| d.as_secs().to_string())
            })
            .unwrap_or_else(|_| "0".to_string());

        let mut parts = Vec::with_capacity(6 + self.context.len() + fields.len());
        parts.push(format!("ts={}", timestamp));
        parts.push(format!("level={}", level_label(level)));
        parts.push(format!("msg=\"{}\"", escape(message)));

        for (k, v) in &self.context {
            parts.push(format!("{}={}", k, escape_value(v)));
        }

        for (k, v) in fields {
            parts.push(format!("{}={}", k, escape_value(v)));
        }

        println!("{}", parts.join(" "));
    }
}

pub fn get_logger(component: &str, spider: Option<&str>) -> Logger {
    let _ = min_level();
    let mut context = Vec::new();
    if !component.is_empty() {
        context.push(("component".to_string(), component.to_string()));
    }
    if let Some(spider) = spider {
        context.push(("spider".to_string(), spider.to_string()));
    }
    Logger { context }
}

pub fn complete_logs() {
    let _ = io::stdout().flush();
}

fn level_label(level: Level) -> &'static str {
    match level {
        Level::Debug => "DEBUG",
        Level::Info => "INFO",
        Level::Warn => "WARN",
        Level::Error => "ERROR",
    }
}

fn escape(text: &str) -> String {
    text.replace('"', "\\\"")
}

fn escape_value(value: &str) -> String {
    if value.is_empty() {
        return "\"\"".to_string();
    }
    let needs_quotes = value.chars().any(|c| c.is_whitespace() || c == '"');
    if needs_quotes {
        format!("\"{}\"", escape(value))
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::{Level, escape, escape_value, level_label};

    #[test]
    fn escape_replaces_quotes() {
        assert_eq!(escape("a\"b"), "a\\\"b");
    }

    #[test]
    fn escape_value_quotes_whitespace() {
        assert_eq!(escape_value("hello world"), "\"hello world\"");
        assert_eq!(escape_value("plain"), "plain");
        assert_eq!(escape_value(""), "\"\"");
    }

    #[test]
    fn level_label_matches_levels() {
        assert_eq!(level_label(Level::Debug), "DEBUG");
        assert_eq!(level_label(Level::Info), "INFO");
        assert_eq!(level_label(Level::Warn), "WARN");
        assert_eq!(level_label(Level::Error), "ERROR");
    }
}
