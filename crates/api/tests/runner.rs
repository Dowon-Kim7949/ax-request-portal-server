//! The runner end to end: the API on a real port, a local bare repository
//! standing in for GitHub's git, a fake GitHub REST API, and a fake `claude`
//! that answers like Claude Code. A report backed by the wiki, the scope
//! check, a work run that pushes a branch and opens a pull request, and a
//! revision that continues both.
#![cfg(unix)]

mod common;

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use axum::extract::{Path as UrlPath, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::{Value, json};
use sqlx::PgPool;

use ax_api::config::{AiSettings, RunnerConfig, Secret};
use ax_api::roles::Role;
use ax_api::runner::Runner;
use ax_api::{AppState, Config, router};
use common::*;

const TOKEN: &str = "runner-token-runner-token-runner-token";

async fn serve(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    url
}

/// The API, taking no AI jobs itself: they wait for a runner.
async fn api(pool: &PgPool, token: Option<&str>) -> String {
    let ai = AiSettings { runner_token: token.map(Secret::new), ..AiSettings::default() };
    let state = AppState { db: Some(pool.clone()), files: files(), ai, ..AppState::default() };
    serve(router(state, &Config::from_lookup(|_| None).unwrap())).await
}

type Created = Arc<Mutex<Vec<Value>>>;

/// GitHub's pull request API for `acme/portal`, remembering what was created.
async fn github() -> (String, Created) {
    let created: Created = Arc::default();
    let list = |State(c): State<Created>| async move {
        let open: Vec<Value> = c.lock().unwrap().iter().map(|p| json!({ "number": p["number"] })).collect();
        Json(json!(open))
    };
    let create = |State(c): State<Created>, Json(mut body): Json<Value>| async move {
        body["number"] = json!(12);
        c.lock().unwrap().push(body);
        (StatusCode::CREATED, Json(json!({ "number": 12 })))
    };
    let one = |UrlPath(n): UrlPath<u64>| async move {
        Json(
            json!({ "number": n, "html_url": format!("https://github.com/acme/portal/pull/{n}"), "changed_files": 2, "additions": 3, "deletions": 0 }),
        )
    };
    let app = Router::new()
        .route("/repos/acme/portal/pulls", get(list).post(create))
        .route("/repos/acme/portal/pulls/{n}", get(one))
        .with_state(Arc::clone(&created));
    (serve(app).await, created)
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git").args(args).current_dir(dir).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// A bare repository with a reviewed wiki page and a source file on `main`.
fn remote(root: &Path) -> PathBuf {
    let bare = root.join("remote.git");
    git(root, &["init", "-q", "--bare", "-b", "main", bare.to_str().unwrap()]);
    let seed = root.join("seed");
    std::fs::create_dir_all(seed.join("docs/llm-wiki")).unwrap();
    std::fs::create_dir_all(seed.join("src")).unwrap();
    std::fs::write(seed.join("docs/llm-wiki/notices.md"), "---\ntitle: Notices\nstatus: verified\n---\n# Notices\n")
        .unwrap();
    // Navigation pages the agent reads first; unreviewed, and no evidence.
    std::fs::write(seed.join("docs/llm-wiki/index.md"), "---\ntitle: Index\nstatus: needs_review\n---\n# Index\n")
        .unwrap();
    std::fs::write(
        seed.join("docs/llm-wiki/start.md"),
        "---\ntitle: Start\nstatus: needs_review\ndoc_type: wiki_index\n---\n# Start\n",
    )
    .unwrap();
    std::fs::write(seed.join("src/notices.txt"), "list\n").unwrap();
    git(&seed, &["init", "-q", "-b", "main"]);
    git(&seed, &["add", "-A"]);
    git(&seed, &["-c", "user.name=t", "-c", "user.email=t@example.com", "commit", "-q", "-m", "seed"]);
    git(&seed, &["push", "-q", bare.to_str().unwrap(), "main"]);
    bare
}

/// A `claude` that reads (report) or edits and answers (work), recording its
/// arguments and key.
fn fake_claude(root: &Path) -> PathBuf {
    let report = json!({ "type": "result", "is_error": false, "result": "done", "total_cost_usd": 0.5,
        "usage": { "input_tokens": 1000, "output_tokens": 200 },
        "structured_output": {
            "verdict": "feasible", "summary": "The list can take a title filter.",
            "wikiDocs": [
                { "path": "docs/llm-wiki/index.md", "title": "Index" },
                { "path": "docs/llm-wiki/start.md", "title": "Start" },
                { "path": "docs/llm-wiki/notices.md", "title": "x" },
                { "path": "docs/llm-wiki/missing.md", "title": "Gone" },
            ],
            "codeFiles": ["src/notices.txt", "src/gone.txt", "../etc/passwd"],
            "plan": ["Add a filter to the list"],
            "verification": [{ "criterionId": "c1", "automatable": true, "method": "A component test types a word" }],
            "openQuestions": [], "assumptions": ["The list is filtered on the server"], "constraintChecks": [],
            "recheck": ["Search"], "size": "S", "risks": [],
            "scope": { "summary": "A search box above the notice list.", "changes": ["A search box"], "unchanged": ["The order"],
                "questions": [{ "text": "Match the body too?", "options": ["Title only", "Title and body"], "recommended": 0 }] },
        } });
    let work = json!({ "type": "result", "is_error": false, "result": "done", "total_cost_usd": 0.5,
        "structured_output": { "outcome": "success", "summary": "Added a title filter.",
            "tests": { "existingPassed": 4, "existingTotal": 4, "criteriaPassed": 1, "criteriaTotal": 1 } } });
    let log = root.join("claude.log");
    let script = format!(
        "#!/usr/bin/env bash\nmode=read\nfor a in \"$@\"; do [ \"$a\" = bypassPermissions ] && mode=edit; done\n\
         echo \"$mode $ANTHROPIC_API_KEY $*\" >> '{log}'\n\
         if [ $mode = read ]; then\ncat <<'JSON'\n{report}\nJSON\nelse\n\
         case \"$*\" in *'Only check it'*) ;; *) echo searchable >> src/notices.txt\n\
         echo 'Search by title.' >> docs/llm-wiki/notices.md;; esac\n\
         cat <<'JSON'\n{work}\nJSON\nfi\n",
        log = log.display()
    );
    let path = root.join("claude");
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// Git over HTTP that refuses to take pushes, as GitHub answers a token
/// without the right to push.
async fn refusing_git() -> String {
    let refuse = || async { (StatusCode::FORBIDDEN, "Write access to repository not granted.\n") };
    serve(Router::new().route("/acme/portal/info/refs", get(refuse))).await
}

/// A runner on the local repository; pushes go to `push` instead, when given.
fn runner(server: &str, github: &str, root: &Path, bare: &Path, claude: &Path, push: Option<&str>) -> Runner {
    let mut config = RunnerConfig::from_lookup(|name| {
        match name {
            "AX_RUNNER_SERVER" => Some(server),
            "AX_RUNNER_TOKEN" => Some(TOKEN),
            "AX_GITHUB_TOKEN" => Some("ghp_test"),
            "AX_ANTHROPIC_API_KEY" => Some("sk-test-default"),
            "AX_ANTHROPIC_API_KEY_WORK" => Some("sk-test-work"),
            _ => None,
        }
        .map(str::to_owned)
    })
    .unwrap();
    config.github_api = github.to_owned();
    config.workdir = root.join("work");
    config.claude = claude.display().to_string();
    config.git_config = vec![
        (format!("url.file://{}.insteadOf", bare.display()), "https://github.com/acme/portal".to_owned()),
        ("protocol.file.allow".to_owned(), "always".to_owned()),
    ];
    if let Some(push) = push {
        config
            .git_config
            .push((format!("url.{push}/acme/portal.pushInsteadOf"), "https://github.com/acme/portal".to_owned()));
    }
    Runner::new(config)
}

async fn request(pool: &PgPool, token: &str) -> Value {
    send(pool, "GET", "/requests/1", Some(token), "en", None).await.1
}

async fn act(pool: &PgPool, token: &str, body: Value) {
    let (status, answer) = send(pool, "POST", "/requests/1/transitions", Some(token), "en", Some(body)).await;
    assert_eq!(status, StatusCode::OK, "{answer}");
}

/// A GitHub project and a change request filed against it; the developer's
/// and the requester's sessions.
async fn filed(pool: &PgPool) -> (String, String) {
    let (_, admin) = signed_in(pool, "admin@example.com", Role::Admin).await;
    let (dev_user, dev) = signed_in(pool, "dev@example.com", Role::Developer).await;
    let (_, requester) = signed_in(pool, "req@example.com", Role::Requester).await;
    let project = json!({
        "name": "Portal", "description": "Notice board", "icon": "globe",
        "repository": { "provider": "github", "url": "https://github.com/acme/portal" },
        "wikiPath": "docs/llm-wiki", "ownerIds": [dev_user.id.to_string()],
        "targets": [{ "name": "Notices" }, { "name": "Search" }], "constraints": [], "active": true,
    });
    let (_, project) = send(pool, "POST", "/projects", Some(&admin), "en", Some(project)).await;
    let filed = json!({
        "projectId": project["id"], "type": "change", "title": "Search notices", "targetIds": [project["targets"][0]["id"]],
        "currentBehavior": "", "desiredBehavior": "Find notices by title",
        "acceptanceCriteria": [{ "text": "When I type a word, matching notices show" }], "attachmentIds": [],
    });
    let (status, _) = send(pool, "POST", "/requests", Some(&requester), "en", Some(filed)).await;
    assert_eq!(status, StatusCode::CREATED);
    (dev, requester)
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn a_runner_reports_with_evidence_then_opens_a_pull_request(pool: PgPool) {
    let root = tempdir();
    let (dev, requester) = filed(&pool).await;
    let server = api(&pool, Some(TOKEN)).await;
    let (github, created) = github().await;
    let bare = remote(&root);
    let claude = fake_claude(&root);
    let runner = runner(&server, &github, &root, &bare, &claude, None);

    // The report, read from the code and the reviewed wiki page.
    assert_eq!(runner.run_once().await, Ok(true));
    let shown = request(&pool, &dev).await;
    assert_eq!(shown["status"], "scope_check", "a feasible report goes to the requester first");
    let report = &shown["report"];
    assert_eq!(report["verdict"], "feasible");
    assert_eq!(report["wikiTrust"], "high");
    assert_eq!(
        report["evidence"],
        json!({ "wikiDocs": [{ "path": "docs/llm-wiki/notices.md", "title": "Notices", "verified": true }], "codeFiles": ["src/notices.txt"] }),
        "only what exists and is about the software (no navigation pages), titled and judged by the page itself"
    );
    assert_eq!(report["plan"], json!(["Add a filter to the list"]));
    assert_eq!(shown["scope"]["proposal"]["questions"][0]["recommended"], "q1o1");
    let log = std::fs::read_to_string(root.join("claude.log")).unwrap();
    assert!(log.starts_with("read sk-test-default -p "), "{log}");
    assert!(log.contains("--disallowedTools Bash,Edit,Write") && log.contains("--bare"));
    let booked: f64 = sqlx::query_scalar("select cost_krw from ai_usage").fetch_one(&pool).await.unwrap();
    assert_eq!(booked, 700.0, "Claude Code's own cost, $0.50 at 1,400");
    let runtime: String = sqlx::query_scalar("select runtime from ai_usage").fetch_one(&pool).await.unwrap();
    assert_eq!(runtime, "claude_code", "a runner's report is a coding-agent run, not one API call");

    // The requester confirms; the developer starts the work.
    act(&pool, &requester, json!({ "event": "approveScope", "choices": [{ "questionId": "q1", "optionId": "q1o1" }] }))
        .await;
    act(&pool, &dev, json!({ "event": "runWork", "checkedAssumptions": ["a1"] })).await;
    assert_eq!(runner.run_once().await, Ok(true));
    let shown = request(&pool, &dev).await;
    assert_eq!(shown["status"], "code_review");
    let work = &shown["work"];
    assert_eq!((&work["outcome"], &work["phase"]), (&json!("success"), &json!("finished")));
    assert_eq!(
        work["mergeRequest"],
        json!({ "number": 12, "url": "https://github.com/acme/portal/pull/12", "filesChanged": 2, "additions": 3, "deletions": 0 })
    );
    assert_eq!(
        work["tests"],
        json!({ "existingPassed": 4, "existingTotal": 4, "criteriaPassed": 1, "criteriaTotal": 1 })
    );
    assert_eq!(work["wikiChanges"], json!({ "docs": ["docs/llm-wiki/notices.md"], "pendingReview": true }));
    assert_eq!(work["costKrw"], 700);
    assert_eq!(shown["regression"][0]["label"], "Search");
    let pr = created.lock().unwrap()[0].clone();
    assert_eq!(
        (&pr["head"], &pr["base"], &pr["title"]),
        (&json!("ax/request-1"), &json!("main"), &json!("[AX #1] Search notices"))
    );
    let bare_dir = bare.parent().unwrap();
    let gd = format!("--git-dir={}", bare.display());
    assert_eq!(git(bare_dir, &[&gd, "log", "-1", "--format=%s", "ax/request-1"]), "[AX #1] Search notices (run 1)");
    let page = git(bare_dir, &[&gd, "show", "ax/request-1:docs/llm-wiki/notices.md"]);
    assert!(page.contains("status: needs_review"), "the AI never leaves its own wiki edit verified: {page}");
    assert_eq!(git(bare_dir, &[&gd, "rev-list", "--count", "main"]), "1", "nothing reaches main");
    let log = std::fs::read_to_string(root.join("claude.log")).unwrap();
    assert!(log.lines().any(|l| l.starts_with("edit sk-test-work -p ")), "the work task's key");
    let edit = &log[log.find("edit sk-test-work").unwrap()..];
    assert!(edit.contains("Title only") && !edit.contains("q1o1"), "the requester's answer, in words: {edit}");

    // A revision continues the branch and the pull request.
    act(&pool, &dev, json!({ "event": "requestRevision", "note": "Also trim spaces" })).await;
    assert_eq!(runner.run_once().await, Ok(true));
    assert_eq!(request(&pool, &dev).await["status"], "code_review");
    assert_eq!(created.lock().unwrap().len(), 1, "the open pull request is reused");
    assert_eq!(git(bare_dir, &[&gd, "rev-list", "--count", "main..ax/request-1"]), "2");
    assert!(std::fs::read_to_string(root.join("claude.log")).unwrap().contains("Also trim spaces"));

    // A run that only checks the pull request changes nothing; its tests pass,
    // so it succeeds, and the request keeps the pull request's link.
    act(&pool, &dev, json!({ "event": "requestRevision", "note": "Only check it" })).await;
    assert_eq!(runner.run_once().await, Ok(true));
    let shown = request(&pool, &dev).await;
    let work = &shown["work"];
    assert_eq!(
        (&shown["status"], &work["outcome"], &work["attempt"]),
        (&json!("code_review"), &json!("success"), &json!(3))
    );
    assert!(work["summary"].as_str().unwrap().starts_with("No file needed changing"), "{work}");
    assert_eq!(work["mergeRequest"]["number"], 12, "the link to the pull request stays");
    assert_eq!(created.lock().unwrap().len(), 1, "none opened");
    assert_eq!(git(bare_dir, &[&gd, "rev-list", "--count", "main..ax/request-1"]), "2", "nothing pushed");
    assert_eq!(runner.run_once().await, Ok(false), "nothing left");
    let booked: Vec<(String, String)> =
        sqlx::query_as("select distinct task, runtime from ai_usage order by task").fetch_all(&pool).await.unwrap();
    assert_eq!(booked, [("report".into(), "claude_code".into()), ("work".into(), "claude_code".into())]);
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn a_token_that_cannot_push_ends_the_work_before_the_agent_runs(pool: PgPool) {
    let root = tempdir();
    let (dev, requester) = filed(&pool).await;
    let server = api(&pool, Some(TOKEN)).await;
    let (github, created) = github().await;
    let bare = remote(&root);
    let claude = fake_claude(&root);
    let refusing = refusing_git().await;
    let runner = runner(&server, &github, &root, &bare, &claude, Some(&refusing));
    assert_eq!(runner.run_once().await, Ok(true), "the report only reads");
    act(&pool, &requester, json!({ "event": "approveScope", "choices": [{ "questionId": "q1", "optionId": "q1o1" }] }))
        .await;
    act(&pool, &dev, json!({ "event": "runWork", "checkedAssumptions": ["a1"] })).await;

    assert_eq!(runner.run_once().await, Ok(true));
    let log = std::fs::read_to_string(root.join("claude.log")).unwrap();
    assert!(!log.lines().any(|l| l.starts_with("edit ")), "the agent never ran: {log}");
    let (status, attempts, error): (String, i32, String) =
        sqlx::query_as("select status, attempts, error from ai_jobs where task = 'work'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!((status.as_str(), attempts), ("failed", 1), "not tried again: it would only refuse again");
    assert!(error.contains("cannot push") && error.contains("403"), "{error}");
    let work_calls: i64 =
        sqlx::query_scalar("select count(*) from ai_usage where task = 'work'").fetch_one(&pool).await.unwrap();
    assert_eq!(work_calls, 0, "nothing spent");
    let shown = request(&pool, &dev).await;
    assert_eq!((&shown["status"], &shown["work"]["outcome"]), (&json!("code_review"), &json!("error")));
    assert!(created.lock().unwrap().is_empty(), "no pull request");
    assert_eq!(runner.run_once().await, Ok(false), "nothing left");
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn runner_routes_need_the_token_and_a_live_claim(pool: PgPool) {
    let call = |url: String, token: Option<&'static str>, body: Value| async move {
        let mut request = reqwest::Client::new().post(url).json(&body);
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        request.send().await.unwrap().status().as_u16()
    };
    let off = api(&pool, None).await;
    assert_eq!(call(format!("{off}/runner/claim"), Some(TOKEN), json!({})).await, 404, "no runner configured");
    let on = api(&pool, Some(TOKEN)).await;
    assert_eq!(call(format!("{on}/runner/claim"), Some("wrong"), json!({})).await, 401);
    assert_eq!(call(format!("{on}/runner/claim"), None, json!({})).await, 401);
    assert_eq!(call(format!("{on}/runner/claim"), Some(TOKEN), json!({})).await, 204, "nothing to do");
    let stale = json!({ "claim": "00000000-0000-0000-0000-000000000000", "report": {} });
    assert_eq!(call(format!("{on}/runner/jobs/1/result"), Some(TOKEN), stale).await, 409);
}

fn tempdir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("ax-runner-test-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}
