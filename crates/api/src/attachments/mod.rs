//! Attachments: files people upload for a request or a draft
//! (`uploadAttachment`), and the signed links that serve them back.
//!
//! The contract has no download operation. An `Attachment` carries a `url`
//! that the browser puts in an `<img>` or a link, and neither can send the
//! bearer token. So the url is a signed link to `GET /files/{id}`, a route
//! outside the contract: an HMAC of the id and an expiry, made afresh each time
//! the server shows an attachment. Only someone who was shown the attachment
//! holds a working link.

pub mod handlers;

use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use argon2::password_hash::rand_core::{OsRng, RngCore};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha2::Sha256;
use sqlx::{PgExecutor, PgPool};
use uuid::Uuid;

/// The contract's limit, "10 MB", counted as the mock API counts it:
/// 10 × 1024 × 1024 bytes, that size included.
pub const MAX_SIZE: u64 = 10 * 1024 * 1024;

/// What `POST /attachments` reads at most: the file and room for the
/// multipart framing. A larger body is cut off and answered `fileTooLarge`.
pub const BODY_LIMIT: usize = 11 * 1024 * 1024;

/// Marks an attachment id.
pub const ID_PREFIX: &str = "att_";

/// File types a browser shows in place without running anything from the
/// file (the frontend's `INLINE_TYPES` in `src/domain/url.ts`). Any other file,
/// SVG and HTML included, is served as a download.
pub const INLINE_TYPES: [&str; 7] =
    ["image/png", "image/jpeg", "image/gif", "image/webp", "image/avif", "image/bmp", "application/pdf"];

/// Links expire on a 12-hour boundary at least 12 hours ahead, so the same
/// attachment keeps the same link for 12 hours (the browser's cache holds)
/// and every link works for 12 to 24 hours.
const WINDOW_SECS: u64 = 12 * 3600;

/// An uploaded file, as the contract's `Attachment` describes it (without `url`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attachment {
    pub id: String,
    pub name: String,
    pub size: i64,
    pub mime_type: String,
}

impl Attachment {
    /// The description a request keeps in its details: the contract's
    /// `Attachment` without `url`, which [`Files::present`] adds when shown.
    pub fn to_json(&self) -> Value {
        json!({ "id": self.id, "name": self.name, "size": self.size, "mimeType": self.mime_type })
    }
}

/// Where uploads are kept and how their links are made.
#[derive(Clone)]
pub struct Files {
    /// `AX_UPLOAD_DIR`: one file per attachment, named by its id.
    pub dir: PathBuf,
    /// `AX_PUBLIC_PATH` without a trailing slash: where the browser reaches
    /// this server (`/api` behind the frontend's nginx; empty for the root).
    public_path: String,
    key: Arc<[u8; 32]>,
}

impl fmt::Debug for Files {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Files").field("dir", &self.dir).field("public_path", &self.public_path).finish_non_exhaustive()
    }
}

impl Default for Files {
    /// Uploads in the system's temporary directory, links under `/api`, and a
    /// random key. The server replaces it with its settings and stored key.
    fn default() -> Self {
        Self::new(std::env::temp_dir().join("ax-api-uploads"), "/api", random_key())
    }
}

impl Files {
    pub fn new(dir: impl Into<PathBuf>, public_path: &str, key: [u8; 32]) -> Self {
        Self { dir: dir.into(), public_path: public_path.trim_end_matches('/').to_owned(), key: Arc::new(key) }
    }

    /// The file that holds attachment `id`.
    pub fn path_of(&self, id: &str) -> PathBuf {
        self.dir.join(id)
    }

    /// The download link for attachment `id`, as the browser reaches it.
    pub fn url(&self, id: &str) -> String {
        self.url_at(id, now())
    }

    fn url_at(&self, id: &str, now: u64) -> String {
        let expires = (now / WINDOW_SECS + 2) * WINDOW_SECS;
        format!("{}/files/{id}?exp={expires}&sig={}", self.public_path, self.signature(id, expires))
    }

    /// Whether `signature` is this server's for `id` and `expires`, and the
    /// link has not expired.
    pub fn verify(&self, id: &str, expires: u64, signature: &str) -> bool {
        self.verify_at(id, expires, signature, now())
    }

    fn verify_at(&self, id: &str, expires: u64, signature: &str, now: u64) -> bool {
        if expires < now {
            return false;
        }
        let Ok(signature) = URL_SAFE_NO_PAD.decode(signature) else { return false };
        self.mac(id, expires).verify_slice(&signature).is_ok()
    }

    fn mac(&self, id: &str, expires: u64) -> Hmac<Sha256> {
        let mut mac = Hmac::<Sha256>::new_from_slice(self.key.as_slice()).expect("HMAC takes a key of any length");
        mac.update(id.as_bytes());
        mac.update(b"\n");
        mac.update(expires.to_string().as_bytes());
        mac
    }

    fn signature(&self, id: &str, expires: u64) -> String {
        URL_SAFE_NO_PAD.encode(self.mac(id, expires).finalize().into_bytes())
    }

    /// A stored description (see [`Attachment::to_json`]) as the contract's
    /// `Attachment`, with a fresh link.
    pub fn present(&self, stored: &Value) -> Value {
        let mut attachment = stored.clone();
        if let Some(fields) = attachment.as_object_mut()
            && let Some(id) = fields.get("id").and_then(Value::as_str).map(str::to_owned)
        {
            fields.insert("url".to_owned(), json!(self.url(&id)));
        }
        attachment
    }
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// 256 random bits, for a key that lives only as long as the process.
pub fn random_key() -> [u8; 32] {
    let mut key = [0u8; 32];
    OsRng.fill_bytes(&mut key);
    key
}

/// Whether `id` looks like an attachment id: the prefix and base64url.
pub fn is_id(id: &str) -> bool {
    id.strip_prefix(ID_PREFIX).is_some_and(|rest| {
        !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    })
}

/// The key that signs download links, made on first start and kept in the
/// database, so links survive restarts and every instance agrees.
///
/// # Errors
///
/// Database failures, or a stored key that is not 32 bytes.
pub async fn signing_key(db: &PgPool) -> Result<[u8; 32], sqlx::Error> {
    sqlx::query("insert into server_secrets (name, value) values ('file_urls', $1) on conflict (name) do nothing")
        .bind(random_key().to_vec())
        .execute(db)
        .await?;
    let key: Vec<u8> =
        sqlx::query_scalar("select value from server_secrets where name = 'file_urls'").fetch_one(db).await?;
    key.try_into().map_err(|_| sqlx::Error::Decode("server_secrets.file_urls is not 32 bytes".into()))
}

/// The attachment with this id, whoever uploaded it.
///
/// # Errors
///
/// Database failures.
pub async fn find(db: impl PgExecutor<'_>, id: &str) -> Result<Option<Attachment>, sqlx::Error> {
    let row: Option<(String, String, i64, String)> =
        sqlx::query_as("select id, name, size, mime_type from attachments where id = $1")
            .bind(id)
            .fetch_optional(db)
            .await?;
    Ok(row.map(|(id, name, size, mime_type)| Attachment { id, name, size, mime_type }))
}

/// The caller's attachments among `ids`, each once, in the order first named
/// (the mock API's `attachmentsOf`), and the indexes of the ids that are not
/// the caller's: missing, or uploaded by someone else.
///
/// # Errors
///
/// Database failures.
pub async fn owned(
    db: impl PgExecutor<'_>,
    owner: Uuid,
    ids: &[String],
) -> Result<(Vec<Attachment>, Vec<usize>), sqlx::Error> {
    if ids.is_empty() {
        return Ok((Vec::new(), Vec::new()));
    }
    let rows: Vec<(String, String, i64, String)> =
        sqlx::query_as("select id, name, size, mime_type from attachments where owner_id = $1 and id = any($2)")
            .bind(owner)
            .bind(ids)
            .fetch_all(db)
            .await?;
    let mut found: Vec<Attachment> = Vec::new();
    let mut missing = Vec::new();
    for (index, id) in ids.iter().enumerate() {
        match rows.iter().find(|row| &row.0 == id) {
            None => missing.push(index),
            Some(_) if found.iter().any(|a| &a.id == id) => {}
            Some((id, name, size, mime_type)) => {
                found.push(Attachment { id: id.clone(), name: name.clone(), size: *size, mime_type: mime_type.clone() })
            }
        }
    }
    Ok((found, missing))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files() -> Files {
        Files::new("/tmp/uploads", "/api/", [7; 32])
    }

    fn parts(url: &str) -> (String, u64, String) {
        let (path, query) = url.split_once('?').unwrap();
        let id = path.strip_prefix("/api/files/").unwrap().to_owned();
        let (exp, sig) = query.strip_prefix("exp=").unwrap().split_once("&sig=").unwrap();
        (id, exp.parse().unwrap(), sig.to_owned())
    }

    #[test]
    fn links_verify_until_they_expire() {
        let f = files();
        let now = 1_000_000_000;
        let (id, exp, sig) = parts(&f.url_at("att_abc", now));
        assert_eq!(id, "att_abc");
        assert!(exp >= now + WINDOW_SECS && exp <= now + 2 * WINDOW_SECS, "12 to 24 hours ahead");
        assert!(f.verify_at(&id, exp, &sig, now));
        assert!(f.verify_at(&id, exp, &sig, exp), "good up to the second it expires");
        assert!(!f.verify_at(&id, exp, &sig, exp + 1));
    }

    #[test]
    fn links_are_bound_to_their_id_expiry_and_key() {
        let f = files();
        let now = 1_000_000_000;
        let (id, exp, sig) = parts(&f.url_at("att_abc", now));
        assert!(!f.verify_at("att_abd", exp, &sig, now));
        assert!(!f.verify_at(&id, exp + WINDOW_SECS, &sig, now));
        assert!(!f.verify_at(&id, exp, "not base64!", now));
        assert!(!f.verify_at(&id, exp, "", now));
        assert!(!Files::new("/tmp/uploads", "/api", [8; 32]).verify_at(&id, exp, &sig, now));
    }

    #[test]
    fn links_stay_the_same_within_a_window() {
        let f = files();
        let start = 40 * WINDOW_SECS;
        assert_eq!(f.url_at("att_abc", start), f.url_at("att_abc", start + WINDOW_SECS - 1));
        assert_ne!(f.url_at("att_abc", start), f.url_at("att_abc", start + WINDOW_SECS));
    }

    #[test]
    fn links_start_at_the_public_path() {
        assert!(Files::new("/u", "/", [0; 32]).url("att_x").starts_with("/files/att_x?exp="));
        assert!(Files::new("/u", "/portal/api", [0; 32]).url("att_x").starts_with("/portal/api/files/att_x?exp="));
    }

    #[test]
    fn presenting_adds_a_link() {
        let stored = Attachment { id: "att_x".into(), name: "a.png".into(), size: 3, mime_type: "image/png".into() };
        let shown = files().present(&stored.to_json());
        assert_eq!(shown["name"], "a.png");
        assert!(shown["url"].as_str().unwrap().starts_with("/api/files/att_x?exp="));
        assert_eq!(files().present(&json!("odd")), json!("odd"));
    }

    #[test]
    fn ids() {
        assert!(is_id("att_AbC-_9"));
        for bad in ["att_", "att_a/b", "att_..", "x_abc", "", "att_a b"] {
            assert!(!is_id(bad), "{bad:?}");
        }
    }

    #[test]
    fn the_debug_output_hides_the_key() {
        assert!(!format!("{:?}", files()).contains('7'));
    }
}
