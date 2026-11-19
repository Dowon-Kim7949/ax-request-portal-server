//! Request bodies: a JSON object checked field by field.
//!
//! Every problem is collected, so a bad body answers one `400` (`checkInput`)
//! with an `issues` entry per field, the way the frontend's mock API reports
//! its schema check. Unknown fields are ignored, as the contract's schemas strip
//! them.

use serde_json::{Map, Value};

use crate::error::{ApiError, Issue};
use crate::lang::Lang;

/// A message in both languages: `(ko, en)`.
pub type Text = (&'static str, &'static str);

/// A field of a partial update (`PATCH`), where `null` means something.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Patch<T> {
    /// Absent: leave the value as it is.
    Keep,
    /// `null`: remove the value.
    Clear,
    Set(T),
}

#[derive(Debug)]
pub struct Body {
    fields: Map<String, Value>,
    issues: Vec<Issue>,
    lang: Lang,
}

impl Body {
    /// Parses a JSON object.
    ///
    /// # Errors
    ///
    /// `400` with one issue at the root (`path: ""`) when the body is not JSON or not an object.
    pub fn parse(bytes: &[u8], lang: Lang) -> Result<Self, ApiError> {
        match serde_json::from_slice::<Value>(bytes) {
            Ok(Value::Object(fields)) => Ok(Self { fields, issues: Vec::new(), lang }),
            _ => {
                let message = lang.pick("JSON 객체를 보내 주세요.", "Send a JSON object.").to_owned();
                Err(ApiError::check_input(lang, vec![Issue { path: String::new(), message }]))
            }
        }
    }

    /// A field that must be present and pass `read`; otherwise an issue is recorded.
    pub fn required<T>(&mut self, key: &str, message: Text, read: impl FnOnce(&Value) -> Option<T>) -> Option<T> {
        let value = self.fields.get(key).and_then(read);
        if value.is_none() {
            self.issue(key, message);
        }
        value
    }

    /// A field that may be absent; when present (`null` included) it must pass `read`.
    pub fn optional<T>(&mut self, key: &str, message: Text, read: impl FnOnce(&Value) -> Option<T>) -> Option<T> {
        let raw = self.fields.get(key)?;
        let value = read(raw);
        if value.is_none() {
            self.issue(key, message);
        }
        value
    }

    /// A field where absent keeps, `null` clears, and anything else must pass `read`.
    pub fn patch<T>(&mut self, key: &str, message: Text, read: impl FnOnce(&Value) -> Option<T>) -> Patch<T> {
        match self.fields.get(key) {
            None => Patch::Keep,
            Some(Value::Null) => Patch::Clear,
            Some(raw) => match read(raw) {
                Some(value) => Patch::Set(value),
                None => {
                    self.issue(key, message);
                    Patch::Keep
                }
            },
        }
    }

    /// The raw value of a field, for checks that go deeper than one level.
    pub fn field(&self, key: &str) -> Option<&Value> {
        self.fields.get(key)
    }

    /// Records a problem at `path` (e.g. `repository.url`, `targets.2.name`).
    pub fn issue(&mut self, path: &str, (ko, en): Text) {
        self.issues.push(Issue { path: path.to_owned(), message: self.lang.pick(ko, en).to_owned() });
    }

    /// Ends the check.
    ///
    /// # Errors
    ///
    /// `400` (`checkInput`) listing every issue recorded.
    pub fn finish(self) -> Result<(), ApiError> {
        if self.issues.is_empty() { Ok(()) } else { Err(ApiError::check_input(self.lang, self.issues)) }
    }
}

/// The length of a string as JavaScript (and zod) counts it.
pub fn utf16_len(text: &str) -> usize {
    text.encode_utf16().count()
}

/// A string, trimmed, with at least one and at most `max` UTF-16 units after
/// trimming (zod's `.trim().min(1).max(max)`, which counts like JavaScript).
pub fn trimmed(value: &Value, max: Option<usize>) -> Option<String> {
    let text = value.as_str()?.trim();
    let len = text.encode_utf16().count();
    (len >= 1 && max.is_none_or(|max| len <= max)).then(|| text.to_owned())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const MSG: Text = ("메시지", "message");

    fn paths(error: ApiError) -> Vec<String> {
        error.body.issues.into_iter().map(|i| i.path).collect()
    }

    #[test]
    fn only_objects_are_bodies() {
        for bad in ["", "{", "[]", "1", "null", "\"x\""] {
            assert_eq!(paths(Body::parse(bad.as_bytes(), Lang::En).unwrap_err()), [""], "{bad:?}");
        }
    }

    fn text(value: &Value) -> Option<String> {
        value.as_str().map(str::to_owned)
    }

    #[test]
    fn collects_every_issue() {
        let mut body = Body::parse(br#"{"a":1,"c":null,"d":"x","unknown":true}"#, Lang::En).unwrap();
        assert_eq!(body.required("a", MSG, text), None);
        assert_eq!(body.required("b", MSG, text), None);
        assert_eq!(body.optional("c", MSG, text), None, "null is not absent");
        assert_eq!(body.optional("missing", MSG, text), None);
        assert_eq!(body.required("d", MSG, text), Some("x".to_owned()));
        assert_eq!(paths(body.finish().unwrap_err()), ["a", "b", "c"]);
    }

    #[test]
    fn patches_tell_absent_from_null() {
        let mut body = Body::parse(br#"{"clear":null,"set":"t","bad":5}"#, Lang::En).unwrap();
        assert_eq!(body.patch("keep", MSG, text), Patch::Keep);
        assert_eq!(body.patch("clear", MSG, text), Patch::Clear);
        assert_eq!(body.patch("set", MSG, text), Patch::Set("t".to_owned()));
        assert_eq!(body.patch("bad", MSG, text), Patch::Keep);
        assert_eq!(paths(body.finish().unwrap_err()), ["bad"]);
    }

    #[test]
    fn trimmed_counts_like_javascript() {
        assert_eq!(trimmed(&json!("  Planning "), Some(40)), Some("Planning".to_owned()));
        assert_eq!(trimmed(&json!("   "), None), None);
        assert_eq!(trimmed(&json!(1), None), None);
        assert_eq!(trimmed(&json!("x".repeat(40)), Some(40)).map(|s| s.len()), Some(40));
        assert_eq!(trimmed(&json!("x".repeat(41)), Some(40)), None);
        assert_eq!(trimmed(&json!("기획".repeat(20)), Some(40)).map(|s| s.chars().count()), Some(40));
        // Outside the BMP a character is two UTF-16 units, as in JavaScript.
        assert_eq!(trimmed(&json!("😀".repeat(21)), Some(40)), None);
    }
}
