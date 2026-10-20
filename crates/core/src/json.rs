//! Addon JSON is written by hundreds of different authors, so fields come as numbers where text is expected and the
//! other way round. These accessors read a field the way the mobile app does (any scalar becomes text) and never fail.

use serde_json::Value;

pub trait JsonExt {
    /// A string field. Numbers and booleans are turned into text; blanks count as missing.
    fn text(&self, key: &str) -> Option<String>;
    fn array(&self, key: &str) -> &[Value];
    fn object(&self, key: &str) -> Option<&Value>;
    fn int(&self, key: &str) -> Option<i64>;
    fn float(&self, key: &str) -> Option<f64>;
    fn flag(&self, key: &str) -> Option<bool>;
    /// An array of strings, skipping everything that is not text.
    fn texts(&self, key: &str) -> Vec<String>;
}

fn scalar_text(value: &Value) -> Option<String> {
    let text = match value {
        Value::String(s) => s.trim().to_owned(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        _ => return None,
    };
    (!text.is_empty()).then_some(text)
}

impl JsonExt for Value {
    fn text(&self, key: &str) -> Option<String> {
        self.get(key).and_then(scalar_text)
    }

    fn array(&self, key: &str) -> &[Value] {
        self.get(key).and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])
    }

    fn object(&self, key: &str) -> Option<&Value> {
        self.get(key).filter(|v| v.is_object())
    }

    fn int(&self, key: &str) -> Option<i64> {
        match self.get(key)? {
            Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
            Value::String(s) => s.trim().parse().ok(),
            _ => None,
        }
    }

    fn float(&self, key: &str) -> Option<f64> {
        match self.get(key)? {
            Value::Number(n) => n.as_f64(),
            Value::String(s) => s.trim().parse().ok(),
            _ => None,
        }
    }

    fn flag(&self, key: &str) -> Option<bool> {
        match self.get(key)? {
            Value::Bool(b) => Some(*b),
            Value::String(s) => match s.as_str() {
                "true" => Some(true),
                "false" => Some(false),
                _ => None,
            },
            _ => None,
        }
    }

    fn texts(&self, key: &str) -> Vec<String> {
        self.array(key).iter().filter_map(scalar_text).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn numbers_read_as_text() {
        let v = json!({ "releaseInfo": 2020, "rating": 8.1, "empty": "  ", "name": " Dune " });
        assert_eq!(v.text("releaseInfo").as_deref(), Some("2020"));
        assert_eq!(v.text("rating").as_deref(), Some("8.1"));
        assert_eq!(v.text("empty"), None);
        assert_eq!(v.text("name").as_deref(), Some("Dune"));
    }

    #[test]
    fn integers_accept_strings() {
        let v = json!({ "season": "2", "episode": 5, "bad": "x" });
        assert_eq!(v.int("season"), Some(2));
        assert_eq!(v.int("episode"), Some(5));
        assert_eq!(v.int("bad"), None);
    }

    #[test]
    fn text_arrays_skip_junk() {
        let v = json!({ "genres": ["Drama", null, 7, {"a":1}, ""] });
        assert_eq!(v.texts("genres"), vec!["Drama".to_owned(), "7".to_owned()]);
    }
}
