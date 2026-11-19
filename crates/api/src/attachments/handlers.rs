//! `uploadAttachment`, and `GET /files/{id}` behind every `Attachment.url`.

use std::path::PathBuf;

use axum::Json;
use axum::extract::multipart::{MultipartError, MultipartRejection};
use axum::extract::{Multipart, Path, RawQuery, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde_json::Value;
use tokio::io::AsyncWriteExt;

use super::{Attachment, Files, ID_PREFIX, INLINE_TYPES, MAX_SIZE};
use crate::auth::{self, CurrentUser, token};
use crate::error::{ApiError, ErrorCode};
use crate::lang::Lang;
use crate::roles::Permission;
use crate::state::AppState;

/// `POST /attachments` (`request:create`): stores the multipart field `file`
/// and answers `201` with its `Attachment`.
///
/// As in the mock API, the file is checked before the permission: `400`
/// (`pickFile`) when there is no file, `400` (`fileTooLarge`) above 10 MB.
/// The name and type are the browser's (`application/octet-stream` when it
/// sends none); serving decides what may open in place.
pub async fn upload(
    State(state): State<AppState>,
    lang: Lang,
    current: CurrentUser,
    multipart: Result<Multipart, MultipartRejection>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let mut upload = receive(&state.files, multipart, lang).await?;
    auth::require(&current.user, Permission::RequestCreate, lang)?;
    let db = state.db(lang)?;

    let attachment = Attachment {
        id: upload.id.clone(),
        name: upload.name.clone(),
        size: i64::try_from(upload.size).map_err(|_| ApiError::internal(lang))?,
        mime_type: upload.mime_type.clone(),
    };
    let path = state.files.path_of(&attachment.id);
    tokio::fs::rename(&upload.temp, &path).await.map_err(|error| io_error(lang, &error))?;
    upload.temp = path;
    sqlx::query("insert into attachments (id, owner_id, name, size, mime_type) values ($1, $2, $3, $4, $5)")
        .bind(&attachment.id)
        .bind(current.user.id)
        .bind(&attachment.name)
        .bind(attachment.size)
        .bind(&attachment.mime_type)
        .execute(db)
        .await
        .map_err(ApiError::db(lang))?;
    upload.kept = true;
    Ok((StatusCode::CREATED, Json(state.files.present(&attachment.to_json()))))
}

/// A file being received. Until it is kept, dropping it deletes the file, so
/// a refused or failed upload leaves nothing behind.
struct Upload {
    temp: PathBuf,
    id: String,
    name: String,
    mime_type: String,
    size: u64,
    kept: bool,
}

impl Drop for Upload {
    fn drop(&mut self) {
        if !self.kept {
            let _ = std::fs::remove_file(&self.temp);
        }
    }
}

/// Reads the first field named `file` into a temporary file in the upload
/// directory, counting its size as it arrives.
async fn receive(
    files: &Files,
    multipart: Result<Multipart, MultipartRejection>,
    lang: Lang,
) -> Result<Upload, ApiError> {
    let mut multipart = multipart.map_err(|_| pick_file(lang))?;
    let mut field = loop {
        match multipart.next_field().await {
            Ok(Some(field)) if field.name() == Some("file") => break field,
            Ok(Some(_)) => {}
            Ok(None) => return Err(pick_file(lang)),
            Err(error) => return Err(multipart_error(lang, &error)),
        }
    };
    // A field without a file name is text, not a file (FormData's rule).
    let Some(name) = field.file_name().map(str::to_owned) else { return Err(pick_file(lang)) };
    let mime_type = field.content_type().filter(|t| !t.is_empty()).unwrap_or("application/octet-stream").to_owned();

    tokio::fs::create_dir_all(&files.dir).await.map_err(|error| io_error(lang, &error))?;
    let id = token::generate_with(ID_PREFIX);
    let mut upload =
        Upload { temp: files.dir.join(format!(".upload-{id}")), id, name, mime_type, size: 0, kept: false };
    let mut out = tokio::fs::File::create(&upload.temp).await.map_err(|error| io_error(lang, &error))?;
    loop {
        match field.chunk().await {
            Ok(Some(chunk)) => {
                upload.size += chunk.len() as u64;
                if upload.size > MAX_SIZE {
                    return Err(file_too_large(lang));
                }
                out.write_all(&chunk).await.map_err(|error| io_error(lang, &error))?;
            }
            Ok(None) => break,
            Err(error) => return Err(multipart_error(lang, &error)),
        }
    }
    out.flush().await.map_err(|error| io_error(lang, &error))?;
    Ok(upload)
}

/// `400` (the mock's `pickFile`).
fn pick_file(lang: Lang) -> ApiError {
    ApiError::new(ErrorCode::BadRequest, lang.pick("파일을 선택해 주세요.", "Choose a file."))
}

/// `400` (the mock's `fileTooLarge`).
fn file_too_large(lang: Lang) -> ApiError {
    ApiError::new(
        ErrorCode::BadRequest,
        lang.pick("10MB 이하 파일만 올릴 수 있습니다.", "Files must be 10 MB or smaller."),
    )
}

/// A body cut off at [`super::BODY_LIMIT`] is a file that is too large; any
/// other broken body has no file to read.
fn multipart_error(lang: Lang, error: &MultipartError) -> ApiError {
    if error.status() == StatusCode::PAYLOAD_TOO_LARGE { file_too_large(lang) } else { pick_file(lang) }
}

fn io_error(lang: Lang, error: &std::io::Error) -> ApiError {
    tracing::error!(%error, "upload storage error");
    ApiError::internal(lang)
}

/// `GET /files/{id}?exp&sig`: the file behind a link the server made (see
/// [`Files::url`]). A missing, altered or expired link is `404`, the same as
/// an unknown file.
///
/// Only the types in [`INLINE_TYPES`] are sent as themselves to open in
/// place; anything else is sent as `application/octet-stream` to download, and
/// `nosniff` stops the browser from guessing otherwise.
pub async fn download(
    State(state): State<AppState>,
    lang: Lang,
    Path(id): Path<String>,
    RawQuery(query): RawQuery,
) -> Result<Response, ApiError> {
    let (mut expires, mut signature) = (None, None);
    for (key, value) in url::form_urlencoded::parse(query.unwrap_or_default().as_bytes()) {
        match key.as_ref() {
            "exp" => expires = value.parse::<u64>().ok(),
            "sig" => signature = Some(value.into_owned()),
            _ => {}
        }
    }
    let signed = match (expires, signature) {
        (Some(expires), Some(signature)) => super::is_id(&id) && state.files.verify(&id, expires, &signature),
        _ => false,
    };
    if !signed {
        return Err(ApiError::not_found(lang));
    }
    let db = state.db(lang)?;
    let attachment =
        super::find(db, &id).await.map_err(ApiError::db(lang))?.ok_or_else(|| ApiError::not_found(lang))?;
    let bytes = match tokio::fs::read(state.files.path_of(&id)).await {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            tracing::warn!(id, "an attachment's file is missing from the upload directory");
            return Err(ApiError::not_found(lang));
        }
        Err(error) => return Err(io_error(lang, &error)),
    };

    let mime_type = attachment.mime_type.to_ascii_lowercase();
    let inline = INLINE_TYPES.contains(&mime_type.as_str());
    let content_type = if inline { mime_type.as_str() } else { "application/octet-stream" };
    let disposition = content_disposition(if inline { "inline" } else { "attachment" }, &attachment.name);
    let headers = [
        (header::CONTENT_TYPE, HeaderValue::from_str(content_type).map_err(|_| ApiError::internal(lang))?),
        (header::CONTENT_DISPOSITION, HeaderValue::from_str(&disposition).map_err(|_| ApiError::internal(lang))?),
        (header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff")),
        (header::CACHE_CONTROL, HeaderValue::from_static("private, max-age=3600")),
    ];
    Ok((headers, bytes).into_response())
}

/// `inline` or `attachment` with the file's name: an ASCII stand-in for old
/// clients and the exact name in RFC 5987 form.
fn content_disposition(kind: &str, name: &str) -> String {
    let plain = |c: char| (c.is_ascii_graphic() && c != '"' && c != '\\') || c == ' ';
    let fallback: String = name.chars().map(|c| if plain(c) { c } else { '_' }).collect();
    let mut encoded = String::new();
    for byte in name.bytes() {
        if byte.is_ascii_alphanumeric() || b"!#$&+-.^_`|~".contains(&byte) {
            encoded.push(char::from(byte));
        } else {
            encoded.push_str(&format!("%{byte:02X}"));
        }
    }
    format!("{kind}; filename=\"{fallback}\"; filename*=UTF-8''{encoded}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispositions_keep_the_name_safely() {
        assert_eq!(
            content_disposition("inline", "a b.png"),
            "inline; filename=\"a b.png\"; filename*=UTF-8''a%20b.png"
        );
        assert_eq!(
            content_disposition("attachment", "화면 \"1\".svg"),
            "attachment; filename=\"__ _1_.svg\"; filename*=UTF-8''%ED%99%94%EB%A9%B4%20%221%22.svg"
        );
        assert!(HeaderValue::from_str(&content_disposition("inline", "x\r\ny")).is_ok(), "no header injection");
    }
}
