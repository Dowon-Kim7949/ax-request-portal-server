//! `listProjects`, `createProject` and `updateProject`, with the frontend mock
//! API's rules (`saveProject` in `src/mocks/engine.ts`).

use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde_json::{Map, Value, json};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use super::Project;
use crate::auth::{self, CurrentUser};
use crate::body::{Body, Text, trimmed, utf16_len};
use crate::error::ApiError;
use crate::lang::Lang;
use crate::roles::Permission;
use crate::state::AppState;

const ICONS: [&str; 5] = ["globe", "smartphone", "layout-dashboard", "server", "box"];
pub(super) const PROVIDERS: [&str; 2] = ["gitlab", "github"];
/// The frontend's `PROJECT_CONSTRAINTS_MAX`.
pub const CONSTRAINTS_MAX: usize = 20;

const NAME: Text = ("프로젝트 이름은 1–60자로 입력해 주세요.", "Enter a project name of 1 to 60 characters.");
const DESCRIPTION: Text = ("설명은 300자 이하로 입력해 주세요.", "Use 300 characters or fewer for the description.");
const ICON: Text = ("아이콘을 확인해 주세요.", "Choose one of the icons.");
const REPOSITORY: Text = ("저장소를 확인해 주세요.", "Check the repository.");
pub(super) const PROVIDER: Text = ("저장소 종류를 확인해 주세요.", "Choose GitLab or GitHub.");
pub(super) const URL: Text = ("http(s)로 시작하는 전체 주소를 입력해 주세요.", "Must be an absolute http(s) URL");
const BRANCH: Text = ("브랜치 이름으로 쓸 수 없는 값입니다.", "This is not a valid branch name.");
/// The frontend's `BRANCH_NAME_MAX`.
pub const BRANCH_NAME_MAX: usize = 200;
const WIKI_PATH: Text = ("위키 경로를 입력해 주세요.", "Enter the wiki path.");
const OWNERS: Text = ("담당자를 한 명 이상 골라 주세요.", "Choose at least one owner.");
const TARGETS: Text = ("대상 목록을 확인해 주세요.", "Check the targets.");
const TARGET: Text = ("대상 항목을 확인해 주세요.", "Check this target.");
const CONSTRAINTS: Text = ("지킬 규칙은 20개까지입니다.", "Keep at most 20 rules.");
const CONSTRAINT: Text = ("규칙을 확인해 주세요.", "Check this rule.");
const CONSTRAINT_TEXT: Text = ("규칙은 1–300자로 입력해 주세요.", "Write the rule in 1 to 300 characters.");
const REFERENCE: Text = ("출처는 60자 이하로 입력해 주세요.", "Use 60 characters or fewer for the source.");
const ACTIVE: Text = ("true 또는 false여야 합니다.", "Must be true or false.");

/// `GET /projects`: every project, oldest first. Anyone signed in may read it,
/// because the request form and every request show projects.
pub async fn list(State(state): State<AppState>, lang: Lang, _: CurrentUser) -> Result<Json<Vec<Project>>, ApiError> {
    let db = state.db(lang)?;
    Ok(Json(Project::all(db).await.map_err(ApiError::db(lang))?))
}

/// `POST /projects` (`project:manage`). `400` (`ownersMustBeDevelopers`) when
/// an owner is not a developer or an admin.
pub async fn create(
    State(state): State<AppState>,
    lang: Lang,
    current: CurrentUser,
    bytes: Bytes,
) -> Result<(StatusCode, Json<Project>), ApiError> {
    let input = ProjectInput::parse(&bytes, lang)?;
    auth::require(&current.user, Permission::ProjectManage, lang)?;
    let db = state.db(lang)?;
    let owners = check_owners(db, &input.owner_ids, lang).await?;

    let mut tx = db.begin().await.map_err(ApiError::db(lang))?;
    let id: Uuid = sqlx::query_scalar(
        "insert into projects (name, description, icon, repository_provider, repository_url, repository_branch,
                               wiki_path, targets, rules, active)
         values ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
         returning id",
    )
    .bind(&input.name)
    .bind(&input.description)
    .bind(&input.icon)
    .bind(&input.provider)
    .bind(&input.url)
    .bind(&input.branch)
    .bind(&input.wiki_path)
    .bind(&input.targets)
    .bind(&input.constraints)
    .bind(input.active)
    .fetch_one(&mut *tx)
    .await
    .map_err(ApiError::db(lang))?;
    set_owners(&mut tx, id, &owners).await.map_err(ApiError::db(lang))?;
    let project = Project::find(&mut *tx, &id.to_string()).await.map_err(ApiError::db(lang))?;
    tx.commit().await.map_err(ApiError::db(lang))?;
    Ok((StatusCode::CREATED, Json(project.ok_or_else(|| ApiError::internal(lang))?)))
}

/// `PUT /projects/{id}` (`project:manage`): replaces the project. Targets and
/// rules sent with an id keep it; new ones get one. The wiki numbers stay. A
/// repository sent without a branch goes back to the default branch.
/// `400` (`ownersMustBeDevelopers`) is checked before `404` (`projectNotFound`),
/// as in the mock API.
pub async fn update(
    State(state): State<AppState>,
    lang: Lang,
    current: CurrentUser,
    Path(id): Path<String>,
    bytes: Bytes,
) -> Result<Json<Project>, ApiError> {
    let input = ProjectInput::parse(&bytes, lang)?;
    auth::require(&current.user, Permission::ProjectManage, lang)?;
    let db = state.db(lang)?;
    let owners = check_owners(db, &input.owner_ids, lang).await?;
    let not_found = || ApiError::project_not_found(lang, StatusCode::NOT_FOUND);
    let id = Uuid::parse_str(&id).map_err(|_| not_found())?;

    let mut tx = db.begin().await.map_err(ApiError::db(lang))?;
    let updated = sqlx::query(
        "update projects
            set name = $2, description = $3, icon = $4, repository_provider = $5, repository_url = $6,
                repository_branch = $7, wiki_path = $8, targets = $9, rules = $10, active = $11, updated_at = now()
          where id = $1",
    )
    .bind(id)
    .bind(&input.name)
    .bind(&input.description)
    .bind(&input.icon)
    .bind(&input.provider)
    .bind(&input.url)
    .bind(&input.branch)
    .bind(&input.wiki_path)
    .bind(&input.targets)
    .bind(&input.constraints)
    .bind(input.active)
    .execute(&mut *tx)
    .await
    .map_err(ApiError::db(lang))?;
    if updated.rows_affected() == 0 {
        return Err(not_found());
    }
    set_owners(&mut tx, id, &owners).await.map_err(ApiError::db(lang))?;
    let project = Project::find(&mut *tx, &id.to_string()).await.map_err(ApiError::db(lang))?;
    tx.commit().await.map_err(ApiError::db(lang))?;
    Ok(Json(project.ok_or_else(not_found)?))
}

/// Every owner must be an existing developer or admin. Repeats are dropped,
/// first place kept.
async fn check_owners(db: &PgPool, ids: &[String], lang: Lang) -> Result<Vec<Uuid>, ApiError> {
    let mut owners: Vec<Uuid> = Vec::new();
    for id in ids {
        let id = Uuid::parse_str(id).map_err(|_| ApiError::owners_must_be_developers(lang))?;
        if !owners.contains(&id) {
            owners.push(id);
        }
    }
    let developers: i64 =
        sqlx::query_scalar("select count(*) from users where id = any($1) and role in ('developer', 'admin')")
            .bind(&owners)
            .fetch_one(db)
            .await
            .map_err(ApiError::db(lang))?;
    if usize::try_from(developers).ok() != Some(owners.len()) {
        return Err(ApiError::owners_must_be_developers(lang));
    }
    Ok(owners)
}

async fn set_owners(tx: &mut Transaction<'_, Postgres>, project: Uuid, owners: &[Uuid]) -> Result<(), sqlx::Error> {
    sqlx::query("delete from project_owners where project_id = $1").bind(project).execute(&mut **tx).await?;
    sqlx::query(
        "insert into project_owners (project_id, user_id, position)
         select $1, owner, (n - 1)::smallint from unnest($2::uuid[]) with ordinality as o(owner, n)",
    )
    .bind(project)
    .bind(owners)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// The contract's `ProjectInput`, checked like its zod schema.
#[derive(Debug)]
struct ProjectInput {
    name: String,
    description: String,
    icon: String,
    provider: String,
    url: String,
    /// `None`: the repository's default branch
    branch: Option<String>,
    wiki_path: String,
    owner_ids: Vec<String>,
    /// `[{ id, name, wikiDoc? }]`, ids filled in
    targets: Value,
    /// `[{ id, text, reference? }]`, ids filled in
    constraints: Value,
    active: bool,
}

impl ProjectInput {
    fn parse(bytes: &[u8], lang: Lang) -> Result<Self, ApiError> {
        let mut body = Body::parse(bytes, lang)?;
        let name = body.required("name", NAME, |v| trimmed(v, Some(60)));
        let description = body
            .required("description", DESCRIPTION, |v| v.as_str().filter(|d| utf16_len(d) <= 300).map(str::to_owned));
        let icon = body.required("icon", ICON, |v| v.as_str().filter(|i| ICONS.contains(i)).map(str::to_owned));
        let (provider, url, branch) = repository(&mut body);
        let wiki_path = body.required("wikiPath", WIKI_PATH, |v| trimmed(v, None));
        let owner_ids = body.required("ownerIds", OWNERS, |v| {
            let ids: Option<Vec<String>> = v.as_array()?.iter().map(|id| id.as_str().map(str::to_owned)).collect();
            ids.filter(|ids| !ids.is_empty())
        });
        let targets = targets(&mut body);
        let constraints = constraints(&mut body);
        let active = body.required("active", ACTIVE, Value::as_bool);
        body.finish()?;
        match (name, description, icon, provider, url, branch, wiki_path, owner_ids, targets, constraints, active) {
            (
                Some(name),
                Some(description),
                Some(icon),
                Some(provider),
                Some(url),
                Some(branch),
                Some(wiki_path),
                Some(owner_ids),
                Some(targets),
                Some(constraints),
                Some(active),
            ) => Ok(Self {
                name,
                description,
                icon,
                provider,
                url,
                branch,
                wiki_path,
                owner_ids,
                targets,
                constraints,
                active,
            }),
            _ => unreachable!("finish() fails when a required field is missing"),
        }
    }
}

/// `repository: { provider, url, branch? }`, with nested issue paths like
/// zod's. The branch is `Some(None)` when it is left out.
fn repository(body: &mut Body) -> (Option<String>, Option<String>, Option<Option<String>>) {
    let Some(repo) = body.field("repository").and_then(Value::as_object).cloned() else {
        body.issue("repository", REPOSITORY);
        return (None, None, None);
    };
    let provider = repo.get("provider").and_then(Value::as_str).filter(|p| PROVIDERS.contains(p)).map(str::to_owned);
    if provider.is_none() {
        body.issue("repository.provider", PROVIDER);
    }
    let url = repo.get("url").and_then(Value::as_str).filter(|u| is_web_url(u)).map(str::to_owned);
    if url.is_none() {
        body.issue("repository.url", URL);
    }
    let branch = match repo.get("branch") {
        None => Some(None),
        Some(v) => v.as_str().and_then(branch_name).map(Some),
    };
    if branch.is_none() {
        body.issue("repository.branch", BRANCH);
    }
    (provider, url, branch)
}

/// The frontend's `BranchName`, trimmed: a name git accepts as a branch
/// (`git check-ref-format --branch`) that can also travel safely as a command
/// argument. 1 to 200 characters; no whitespace or control characters, none
/// of `~ ^ : ? * [ \`, no `..`, `//` or `@{`, no part that starts with `.` or
/// ends with `.lock`, no leading `-` or `/`, no trailing `/` or `.`, and not
/// `@` alone. The contract writes it as a pattern with lookaheads, which the
/// `regex` crate does not take, so the rules are spelled out here.
pub fn branch_name(value: &str) -> Option<String> {
    let name = value.trim();
    let ok = (1..=BRANCH_NAME_MAX).contains(&utf16_len(name))
        && !name.starts_with(['-', '/', '.'])
        && !name.ends_with(['/', '.'])
        && name != "@"
        && !["..", "//", "/.", "@{"].iter().any(|bad| name.contains(bad))
        && !name.split('/').any(|part| part.ends_with(".lock"))
        && !name.chars().any(|c| c <= ' ' || c == '\u{7f}' || "~^:?*[\\".contains(c));
    ok.then(|| name.to_owned())
}

/// `targets: [{ id?, name, wikiDoc? }]`; a target without an id gets one.
fn targets(body: &mut Body) -> Option<Value> {
    let Some(list) = body.field("targets").and_then(Value::as_array).cloned() else {
        body.issue("targets", TARGETS);
        return None;
    };
    let mut out = Vec::with_capacity(list.len());
    let mut ok = true;
    for (i, target) in list.iter().enumerate() {
        let id = optional_string(target.get("id"));
        let name = target.get("name").and_then(Value::as_str);
        let wiki_doc = optional_string(target.get("wikiDoc"));
        match (target.is_object(), id, name, wiki_doc) {
            (true, Some(id), Some(name), Some(wiki_doc)) => {
                let mut t = Map::new();
                t.insert("id".to_owned(), Value::from(id.unwrap_or_else(|| Uuid::new_v4().to_string())));
                t.insert("name".to_owned(), Value::from(name));
                if let Some(doc) = wiki_doc {
                    t.insert("wikiDoc".to_owned(), Value::from(doc));
                }
                out.push(Value::Object(t));
            }
            _ => {
                body.issue(&format!("targets.{i}"), TARGET);
                ok = false;
            }
        }
    }
    ok.then_some(Value::Array(out))
}

/// `constraints: [{ id?, text, reference? }]` (at most 20); text and reference
/// are trimmed, an empty reference dropped, as in the mock API.
fn constraints(body: &mut Body) -> Option<Value> {
    let Some(list) = body.field("constraints").and_then(Value::as_array).cloned() else {
        body.issue("constraints", CONSTRAINTS);
        return None;
    };
    if list.len() > CONSTRAINTS_MAX {
        body.issue("constraints", CONSTRAINTS);
        return None;
    }
    let mut out = Vec::with_capacity(list.len());
    let mut ok = true;
    for (i, rule) in list.iter().enumerate() {
        if !rule.is_object() {
            body.issue(&format!("constraints.{i}"), CONSTRAINT);
            ok = false;
            continue;
        }
        let id = optional_string(rule.get("id"));
        let text = rule.get("text").and_then(|t| trimmed(t, Some(300)));
        let reference = match rule.get("reference") {
            None => Some(None),
            Some(r) => r.as_str().map(str::trim).filter(|r| utf16_len(r) <= 60).map(|r| Some(r.to_owned())),
        };
        if id.is_none() {
            body.issue(&format!("constraints.{i}.id"), CONSTRAINT);
        }
        if text.is_none() {
            body.issue(&format!("constraints.{i}.text"), CONSTRAINT_TEXT);
        }
        if reference.is_none() {
            body.issue(&format!("constraints.{i}.reference"), REFERENCE);
        }
        let (Some(id), Some(text), Some(reference)) = (id, text, reference) else {
            ok = false;
            continue;
        };
        let mut r = json!({ "id": id.unwrap_or_else(|| Uuid::new_v4().to_string()), "text": text });
        if let Some(reference) = reference.filter(|r| !r.is_empty()) {
            r["reference"] = Value::from(reference);
        }
        out.push(r);
    }
    ok.then_some(Value::Array(out))
}

/// Absent → `Some(None)`; a string → `Some(Some(s))`; anything else → `None`.
fn optional_string(value: Option<&Value>) -> Option<Option<String>> {
    match value {
        None => Some(None),
        Some(v) => v.as_str().map(|s| Some(s.to_owned())),
    }
}

/// The frontend's `WebUrl`: an absolute http(s) URL with a host, and no
/// spaces, control characters or backslashes that browsers read differently.
pub fn is_web_url(value: &str) -> bool {
    if value.chars().any(|c| c <= ' ' || c == '\u{7f}' || c == '\\') {
        return false;
    }
    url::Url::parse(value)
        .is_ok_and(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some_and(|h| !h.is_empty()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_urls() {
        for ok in ["https://git.example.com/team/app", "http://localhost:8080/x"] {
            assert!(is_web_url(ok), "{ok}");
        }
        for bad in [
            "",
            "/team/app",
            "git@example.com:team/app.git",
            "ftp://example.com",
            "https://",
            "https://a b.com",
            "https:\\\\x",
        ] {
            assert!(!is_web_url(bad), "{bad:?}");
        }
    }

    /// The cases of the frontend's `src/domain/branch-name.test.ts`.
    #[test]
    fn branch_names_follow_git() {
        for ok in ["main", "develop", "csap-docker", "release/2026.11", "feature/notice_search", "v1.2", "a"] {
            assert_eq!(branch_name(ok).as_deref(), Some(ok), "{ok}");
        }
        assert_eq!(branch_name("  develop ").as_deref(), Some("develop"), "trimmed");
        let long = "x".repeat(BRANCH_NAME_MAX + 1);
        for bad in [
            "",
            " ",
            "-delete",
            "/main",
            "main/",
            ".hidden",
            "feature/.hidden",
            "main.",
            "a..b",
            "a//b",
            "a@{1}",
            "@",
            "main.lock",
            "main.lock/x",
            "with space",
            "tab\there",
            "a~1",
            "a^",
            "a:b",
            "a?",
            "a*",
            "a[b",
            "a\\b",
            &long,
        ] {
            assert_eq!(branch_name(bad), None, "{bad:?}");
        }
    }

    fn issues(body: &str) -> Vec<String> {
        ProjectInput::parse(body.as_bytes(), Lang::En).unwrap_err().body.issues.into_iter().map(|i| i.path).collect()
    }

    #[test]
    fn checks_the_input_like_zod() {
        let ok = json!({
            "name": "  Portal ", "description": "", "icon": "globe",
            "repository": { "provider": "gitlab", "url": "https://git.example.com/p" },
            "wikiPath": " docs/llm-wiki ", "ownerIds": ["x"],
            "targets": [{ "name": "Home" }, { "id": "t-1", "name": "Notice", "wikiDoc": "notice.md" }],
            "constraints": [{ "text": " No PII in logs ", "reference": " " }, { "id": "c-1", "text": "Keep the API", "reference": "Policy 3" }],
            "active": true
        });
        let input = ProjectInput::parse(ok.to_string().as_bytes(), Lang::En).unwrap();
        assert_eq!((input.name.as_str(), input.wiki_path.as_str()), ("Portal", "docs/llm-wiki"));
        assert_eq!(input.targets[1], json!({ "id": "t-1", "name": "Notice", "wikiDoc": "notice.md" }));
        assert!(input.targets[0]["id"].as_str().is_some_and(|id| Uuid::parse_str(id).is_ok()));
        assert_eq!(input.constraints[0]["text"], "No PII in logs");
        assert!(input.constraints[0].get("reference").is_none(), "an empty reference is dropped");
        assert_eq!(input.constraints[1], json!({ "id": "c-1", "text": "Keep the API", "reference": "Policy 3" }));

        assert_eq!(
            issues(
                r#"{"repository":{"provider":"svn","url":"x","branch":"a..b"},"ownerIds":[],"targets":[{"id":1}],"constraints":[{"text":""}]}"#
            ),
            [
                "name",
                "description",
                "icon",
                "repository.provider",
                "repository.url",
                "repository.branch",
                "wikiPath",
                "ownerIds",
                "targets.0",
                "constraints.0.text",
                "active"
            ]
        );
        let many = json!({ "constraints": vec![json!({ "text": "x" }); 21] });
        assert!(issues(&many.to_string()).contains(&"constraints".to_owned()));
    }
}
