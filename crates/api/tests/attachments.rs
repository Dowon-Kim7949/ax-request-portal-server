//! Attachments against a real PostgreSQL: `uploadAttachment`, the signed
//! download links in `Attachment.url`, and attachments on a filed request.

mod common;

use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;

use ax_api::attachments::MAX_SIZE;
use ax_api::roles::Role;
use common::*;

const PNG: &[u8] = b"\x89PNG\r\n\x1a\nnot really a picture";

enum Part<'a> {
    File { filename: &'a str, mime: Option<&'a str>, bytes: &'a [u8] },
    Text(&'a str),
}

/// `POST /attachments` with the multipart parts given, each named `file`
/// unless it is `("other", ...)`.
async fn upload(pool: &PgPool, token: Option<&str>, lang: &str, parts: &[(&str, Part<'_>)]) -> (StatusCode, Value) {
    let mut body = Vec::new();
    for (name, part) in parts {
        body.extend_from_slice(b"--BOUNDARY\r\n");
        match part {
            Part::File { filename, mime, bytes } => {
                body.extend_from_slice(
                    format!("Content-Disposition: form-data; name=\"{name}\"; filename=\"{filename}\"\r\n").as_bytes(),
                );
                if let Some(mime) = mime {
                    body.extend_from_slice(format!("Content-Type: {mime}\r\n").as_bytes());
                }
                body.extend_from_slice(b"\r\n");
                body.extend_from_slice(bytes);
            }
            Part::Text(value) => {
                body.extend_from_slice(format!("Content-Disposition: form-data; name=\"{name}\"\r\n\r\n").as_bytes());
                body.extend_from_slice(value.as_bytes());
            }
        }
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(b"--BOUNDARY--\r\n");
    let mut request = Request::builder()
        .method("POST")
        .uri("/attachments")
        .header("accept-language", lang)
        .header("content-type", "multipart/form-data; boundary=BOUNDARY");
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    let response = app(pool).oneshot(request.body(Body::from(body)).unwrap()).await.unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap())
}

async fn upload_file(pool: &PgPool, token: &str, filename: &str, mime: Option<&str>, bytes: &[u8]) -> Value {
    let (status, body) = upload(pool, Some(token), "en", &[("file", Part::File { filename, mime, bytes })]).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    body
}

/// Follows a link as the browser would, without a token. The test app is
/// mounted at `/`, behind what would be the frontend's `/api`.
async fn download(pool: &PgPool, url: &str) -> (StatusCode, HeaderMap, Vec<u8>) {
    let path = url.strip_prefix("/api").expect("links start at AX_PUBLIC_PATH");
    let response = app(pool).oneshot(Request::get(path).body(Body::empty()).unwrap()).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.unwrap().to_bytes().to_vec();
    (status, headers, bytes)
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> &'a str {
    headers.get(name).map_or("", |v| v.to_str().unwrap())
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn an_upload_is_served_back_through_its_link(pool: PgPool) {
    let (_, requester) = signed_in(&pool, "req@example.com", Role::Requester).await;
    let attachment = upload_file(&pool, &requester, "화면 1.png", Some("image/png"), PNG).await;
    let id = attachment["id"].as_str().unwrap();
    assert!(id.starts_with("att_"), "{attachment}");
    assert_eq!(attachment["name"], "화면 1.png");
    assert_eq!(attachment["size"], PNG.len());
    assert_eq!(attachment["mimeType"], "image/png");
    let url = attachment["url"].as_str().unwrap();
    assert!(url.starts_with(&format!("/api/files/{id}?exp=")), "{url}");
    assert_eq!(attachment.as_object().unwrap().len(), 5, "ownerId is never shown: {attachment}");

    let (status, headers, bytes) = download(&pool, url).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(bytes, PNG);
    assert_eq!(header(&headers, "content-type"), "image/png");
    assert_eq!(
        header(&headers, "content-disposition"),
        "inline; filename=\"__ 1.png\"; filename*=UTF-8''%ED%99%94%EB%A9%B4%201.png"
    );
    assert_eq!(header(&headers, "x-content-type-options"), "nosniff");

    // A link that is not exactly the server's finds nothing.
    let (path, signature) = url.rsplit_once("&sig=").unwrap();
    let flipped = if signature.ends_with('A') { "B" } else { "A" };
    let altered = format!("{path}&sig={}{flipped}", &signature[..signature.len() - 1]);
    let other_file = url.replacen(id, "att_someoneelse", 1);
    let later = url.replacen("?exp=", "?exp=9", 1);
    for bad in [altered, other_file, later, format!("/api/files/{id}")] {
        let (status, _, bytes) = download(&pool, &bad).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{bad}");
        assert_eq!(serde_json::from_slice::<Value>(&bytes).unwrap()["code"], "not_found");
    }
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn only_safe_types_open_in_place(pool: PgPool) {
    let (_, requester) = signed_in(&pool, "req@example.com", Role::Requester).await;
    let svg = upload_file(&pool, &requester, "logo.svg", Some("image/svg+xml"), b"<svg onload=alert(1)/>").await;
    assert_eq!(svg["mimeType"], "image/svg+xml", "the browser's type is reported as sent");
    let (status, headers, _) = download(&pool, svg["url"].as_str().unwrap()).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(header(&headers, "content-type"), "application/octet-stream");
    assert!(header(&headers, "content-disposition").starts_with("attachment;"));

    let untyped = upload_file(&pool, &requester, "notes", None, b"plain").await;
    assert_eq!(untyped["mimeType"], "application/octet-stream");
    let pdf = upload_file(&pool, &requester, "spec.pdf", Some("Application/PDF"), b"%PDF-1.7").await;
    let (_, headers, _) = download(&pool, pdf["url"].as_str().unwrap()).await;
    assert_eq!(header(&headers, "content-type"), "application/pdf");
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn uploads_follow_the_mock_apis_rules(pool: PgPool) {
    let (_, requester) = signed_in(&pool, "req@example.com", Role::Requester).await;
    let (_, operator) = signed_in(&pool, "ops@example.com", Role::Operator).await;
    let file = |bytes| ("file", Part::File { filename: "a.png", mime: Some("image/png"), bytes });

    let (status, body) = upload(&pool, None, "en", &[file(PNG)]).await;
    assert_eq!((status, body), (StatusCode::UNAUTHORIZED, signed_out("en")));

    let pick = json!({ "code": "bad_request", "message": "Choose a file." });
    assert_eq!(upload(&pool, Some(&requester), "en", &[]).await, (StatusCode::BAD_REQUEST, pick.clone()));
    assert_eq!(
        upload(&pool, Some(&requester), "en", &[("other", Part::Text("x")), ("file", Part::Text("x"))]).await,
        (StatusCode::BAD_REQUEST, pick.clone()),
        "a text field is not a file"
    );
    assert_eq!(
        send(&pool, "POST", "/attachments", Some(&requester), "en", Some(json!({ "file": "x" }))).await,
        (StatusCode::BAD_REQUEST, pick),
        "not multipart"
    );
    let (status, body) = upload(&pool, Some(&requester), "ko", &[]).await;
    assert_eq!((status, body["message"].as_str()), (StatusCode::BAD_REQUEST, Some("파일을 선택해 주세요.")));

    // The file is checked before the permission, as in the mock API.
    let (status, _) = upload(&pool, Some(&operator), "en", &[]).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let (status, body) = upload(&pool, Some(&operator), "en", &[file(PNG)]).await;
    assert_eq!((status, body["code"].as_str()), (StatusCode::FORBIDDEN, Some("forbidden")));

    let limit = usize::try_from(MAX_SIZE).unwrap();
    let too_large = vec![0u8; limit + 1];
    assert_eq!(
        upload(&pool, Some(&requester), "en", &[file(&too_large)]).await,
        (StatusCode::BAD_REQUEST, json!({ "code": "bad_request", "message": "Files must be 10 MB or smaller." }))
    );
    let (status, body) = upload(&pool, Some(&requester), "en", &[file(&too_large[..limit])]).await;
    assert_eq!((status, body["size"].as_u64()), (StatusCode::CREATED, Some(MAX_SIZE)), "10 MB itself is allowed");

    let count: i64 = sqlx::query_scalar("select count(*) from attachments").fetch_one(&pool).await.unwrap();
    assert_eq!(count, 1, "only the accepted upload is kept");
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn a_request_carries_the_requesters_attachments(pool: PgPool) {
    let (_, admin) = signed_in(&pool, "admin@example.com", Role::Admin).await;
    let (owner, _) = signed_in(&pool, "dev@example.com", Role::Developer).await;
    let (_, requester) = signed_in(&pool, "req@example.com", Role::Requester).await;
    let (_, someone_else) = signed_in(&pool, "req2@example.com", Role::Requester).await;
    let body = json!({
        "name": "Portal", "description": "", "icon": "globe",
        "repository": { "provider": "gitlab", "url": "https://git.example.com/portal" },
        "wikiPath": "docs/llm-wiki", "ownerIds": [owner.id.to_string()],
        "targets": [], "constraints": [], "active": true,
    });
    let (status, project) = send(&pool, "POST", "/projects", Some(&admin), "en", Some(body)).await;
    assert_eq!(status, StatusCode::CREATED, "{project}");
    let draft = |ids: Value| {
        json!({
            "projectId": project["id"], "type": "change", "title": "Show the error", "targetIds": [],
            "currentBehavior": "", "desiredBehavior": "Say what went wrong",
            "acceptanceCriteria": [{ "text": "When saving fails, the reason is shown" }], "attachmentIds": ids,
        })
    };
    let a = upload_file(&pool, &requester, "a.png", Some("image/png"), PNG).await;
    let b = upload_file(&pool, &requester, "b.pdf", Some("application/pdf"), b"%PDF").await;
    let theirs = upload_file(&pool, &someone_else, "c.png", Some("image/png"), PNG).await;

    let (status, body) = send(
        &pool,
        "POST",
        "/requests",
        Some(&requester),
        "en",
        Some(draft(json!([a["id"], theirs["id"], "att_gone"]))),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["message"], "An attachment was not found. Remove it and upload it again.");
    let paths: Vec<&str> = body["issues"].as_array().unwrap().iter().map(|i| i["path"].as_str().unwrap()).collect();
    assert_eq!(paths, ["attachmentIds.1", "attachmentIds.2"]);

    let (status, request) =
        send(&pool, "POST", "/requests", Some(&requester), "en", Some(draft(json!([b["id"], a["id"], b["id"]])))).await;
    assert_eq!(status, StatusCode::CREATED, "{request}");
    let shown = request["attachments"].as_array().unwrap();
    let names: Vec<&str> = shown.iter().map(|x| x["name"].as_str().unwrap()).collect();
    assert_eq!(names, ["b.pdf", "a.png"], "each once, in the order given");
    assert_eq!(shown[1]["id"], a["id"]);
    assert_eq!(shown[1]["size"], PNG.len());

    let path = format!("/requests/{}", request["id"]);
    let (status, request) = send(&pool, "GET", &path, Some(&admin), "en", None).await;
    assert_eq!(status, StatusCode::OK);
    let (status, _, bytes) = download(&pool, request["attachments"][1]["url"].as_str().unwrap()).await;
    assert_eq!((status, bytes.as_slice()), (StatusCode::OK, PNG));
}
