//! The response language, from `Accept-Language`.
//!
//! The frontend sends the language the user picked (`ko` or `en`) on every
//! request, and expects error messages, notifications and AI-written text back
//! in that language. Anything else falls back to Korean, the template's default.

use std::convert::Infallible;

use axum::extract::FromRequestParts;
use axum::http::header::ACCEPT_LANGUAGE;
use axum::http::request::Parts;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Lang {
    #[default]
    Ko,
    En,
}

impl Lang {
    /// Picks the supported language with the highest `q` (earliest on a tie).
    pub fn from_header(value: Option<&str>) -> Self {
        let mut best: Option<(Self, f32)> = None;
        for range in value.unwrap_or_default().split(',') {
            let mut parts = range.split(';');
            let tag = parts.next().unwrap_or_default().trim().to_ascii_lowercase();
            let primary = tag.split('-').next().unwrap_or_default();
            let lang = match primary {
                "ko" => Self::Ko,
                "en" => Self::En,
                _ => continue,
            };
            let q =
                parts.filter_map(|p| p.trim().strip_prefix("q=")).find_map(|q| q.parse::<f32>().ok()).unwrap_or(1.0);
            if q > 0.0 && best.is_none_or(|(_, best_q)| q > best_q) {
                best = Some((lang, q));
            }
        }
        best.map_or_else(Self::default, |(lang, _)| lang)
    }

    /// The language of a request, from its headers.
    pub fn of(headers: &axum::http::HeaderMap) -> Self {
        Self::from_header(headers.get(ACCEPT_LANGUAGE).and_then(|v| v.to_str().ok()))
    }

    /// The text for this language.
    pub const fn pick<'a>(self, ko: &'a str, en: &'a str) -> &'a str {
        match self {
            Self::Ko => ko,
            Self::En => en,
        }
    }
}

impl<S: Send + Sync> FromRequestParts<S> for Lang {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(Self::of(&parts.headers))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_values_from_the_frontend() {
        assert_eq!(Lang::from_header(Some("ko")), Lang::Ko);
        assert_eq!(Lang::from_header(Some("en")), Lang::En);
    }

    #[test]
    fn browser_style_lists() {
        assert_eq!(Lang::from_header(Some("en-US,en;q=0.9,ko;q=0.8")), Lang::En);
        assert_eq!(Lang::from_header(Some("fr;q=1, ko-KR;q=0.7, en;q=0.5")), Lang::Ko);
        assert_eq!(Lang::from_header(Some("ko;q=0.5, en;q=0.5")), Lang::Ko);
    }

    #[test]
    fn falls_back_to_korean() {
        assert_eq!(Lang::from_header(None), Lang::Ko);
        assert_eq!(Lang::from_header(Some("fr, de")), Lang::Ko);
        assert_eq!(Lang::from_header(Some("en;q=0")), Lang::Ko);
    }
}
