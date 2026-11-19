//! Sign-in against a real PostgreSQL: `login`, `me`, `logout`, sessions and
//! account provisioning.
//!
//! Needs `DATABASE_URL` (a server where the user may create databases):
//! `scripts/cargo.sh test` and CI provide one. Each test gets its own fresh,
//! migrated database.

mod common;

use axum::http::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;

use ax_api::accounts::{self, AccountError, NewAccount};
use ax_api::auth::token;
use ax_api::db;
use ax_api::roles::Role;
use common::*;

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn sign_in_look_up_and_sign_out(pool: PgPool) {
    let user = account(&pool, "ada@example.com", Role::Admin).await;

    let (status, body) = login(&pool, "ada@example.com", PASSWORD, "en").await;
    assert_eq!(status, StatusCode::OK);
    let token = body["token"].as_str().unwrap().to_owned();
    assert!(token.starts_with(token::PREFIX), "{token}");
    let expected = json!({
        "id": user.id.to_string(),
        "name": "Ada Lovelace",
        "email": "ada@example.com",
        "role": "admin",
        "active": true,
    });
    assert_eq!(body["user"], expected, "no teamId key when there is no team");

    assert_eq!(me(&pool, &token).await, (StatusCode::OK, expected));

    let (status, body) = send(&pool, "POST", "/auth/logout", Some(&token), "en", None).await;
    assert_eq!((status, body), (StatusCode::NO_CONTENT, Value::Null));

    assert_eq!(me(&pool, &token).await, (StatusCode::UNAUTHORIZED, signed_out("en")));
    let (status, _) = send(&pool, "POST", "/auth/logout", Some(&token), "en", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn every_sign_in_failure_looks_the_same(pool: PgPool) {
    account(&pool, "ada@example.com", Role::Requester).await;
    account(&pool, "gone@example.com", Role::Requester).await;
    sqlx::query("update users set active = false where email = 'gone@example.com'").execute(&pool).await.unwrap();
    sqlx::query("insert into users (email, name, role) values ('new@example.com', 'New', 'developer')")
        .execute(&pool)
        .await
        .unwrap();

    for (lang, message) in
        [("ko", "이메일 또는 비밀번호가 맞지 않습니다."), ("en", "The email or password is incorrect.")]
    {
        let expected = (StatusCode::UNAUTHORIZED, json!({ "code": "unauthorized", "message": message }));
        // wrong password, no such account, deactivated account, no password set yet
        assert_eq!(login(&pool, "ada@example.com", "correct horse battery stapl", lang).await, expected);
        assert_eq!(login(&pool, "nobody@example.com", PASSWORD, lang).await, expected);
        assert_eq!(login(&pool, "gone@example.com", PASSWORD, lang).await, expected);
        assert_eq!(login(&pool, "new@example.com", PASSWORD, lang).await, expected);
    }
    let sessions: i64 = sqlx::query_scalar("select count(*) from sessions").fetch_one(&pool).await.unwrap();
    assert_eq!(sessions, 0);
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn emails_match_in_any_case(pool: PgPool) {
    account(&pool, "Ada@Example.com", Role::Developer).await;
    let (status, body) = login(&pool, "ada@EXAMPLE.COM", PASSWORD, "en").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["user"]["email"], "Ada@Example.com", "the email is kept as entered");
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn a_bad_body_is_400_check_input(pool: PgPool) {
    let (status, body) =
        send(&pool, "POST", "/auth/login", None, "ko", Some(json!({ "email": " ada@example.com" }))).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["code"], "bad_request");
    assert_eq!(body["message"], "입력값을 확인해 주세요.");
    let paths: Vec<&str> = body["issues"].as_array().unwrap().iter().map(|i| i["path"].as_str().unwrap()).collect();
    assert_eq!(paths, ["email", "password"]);
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn unknown_tokens_are_401(pool: PgPool) {
    for token in ["nope", "axs_", "axs_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"] {
        assert_eq!(me(&pool, token).await, (StatusCode::UNAUTHORIZED, signed_out("en")), "{token}");
    }
    let (status, body) = send(&pool, "GET", "/auth/me", Some("nope"), "ko", None).await;
    assert_eq!((status, body), (StatusCode::UNAUTHORIZED, signed_out("ko")));
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn deactivation_ends_sign_ins_at_once(pool: PgPool) {
    account(&pool, "ada@example.com", Role::Executive).await;
    let token = token_for(&pool, "ada@example.com").await;
    assert_eq!(me(&pool, &token).await.0, StatusCode::OK);

    sqlx::query("update users set active = false").execute(&pool).await.unwrap();
    assert_eq!(me(&pool, &token).await, (StatusCode::UNAUTHORIZED, signed_out("en")));
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn ended_sessions_are_401(pool: PgPool) {
    account(&pool, "ada@example.com", Role::Requester).await;
    let token = token_for(&pool, "ada@example.com").await;
    sqlx::query("update sessions set expires_at = now() - interval '1 second'").execute(&pool).await.unwrap();
    assert_eq!(me(&pool, &token).await, (StatusCode::UNAUTHORIZED, signed_out("en")));

    // Signing in again clears ended sessions out.
    token_for(&pool, "ada@example.com").await;
    let sessions: i64 = sqlx::query_scalar("select count(*) from sessions").fetch_one(&pool).await.unwrap();
    assert_eq!(sessions, 1);
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn use_moves_the_end_ahead_but_not_past_the_absolute_end(pool: PgPool) {
    account(&pool, "ada@example.com", Role::Requester).await;
    let token = token_for(&pool, "ada@example.com").await;
    let idle_left = "select extract(epoch from expires_at - now())::float8 from sessions";

    // Last used two hours ago, one hour left: using it restores the full 24 hours.
    sqlx::query(
        "update sessions set last_seen_at = now() - interval '2 hours', expires_at = now() + interval '1 hour'",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(me(&pool, &token).await.0, StatusCode::OK);
    let left: f64 = sqlx::query_scalar(idle_left).fetch_one(&pool).await.unwrap();
    assert!((23.9 * 3600.0..=24.0 * 3600.0).contains(&left), "{left}");

    // Used again within the minute: no write.
    let seen = "select last_seen_at::text from sessions";
    let before: String = sqlx::query_scalar(seen).fetch_one(&pool).await.unwrap();
    assert_eq!(me(&pool, &token).await.0, StatusCode::OK);
    let after: String = sqlx::query_scalar(seen).fetch_one(&pool).await.unwrap();
    assert_eq!(before, after);

    // Near the absolute end, the end stops there.
    sqlx::query(
        "update sessions set last_seen_at = now() - interval '2 hours',
                             expires_at = now() + interval '10 minutes',
                             absolute_expires_at = now() + interval '10 minutes'",
    )
    .execute(&pool)
    .await
    .unwrap();
    assert_eq!(me(&pool, &token).await.0, StatusCode::OK);
    let left: f64 = sqlx::query_scalar(idle_left).fetch_one(&pool).await.unwrap();
    assert!(left <= 600.0, "{left}");
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn only_the_tokens_hash_is_stored(pool: PgPool) {
    account(&pool, "ada@example.com", Role::Requester).await;
    let token = token_for(&pool, "ada@example.com").await;
    let stored: Vec<u8> = sqlx::query_scalar("select token_hash from sessions").fetch_one(&pool).await.unwrap();
    assert_eq!(stored, token::digest(&token));
    let plain: i64 = sqlx::query_scalar("select count(*) from sessions where token_hash = convert_to($1, 'UTF8')")
        .bind(&token)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(plain, 0);
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn me_names_the_team(pool: PgPool) {
    let user = account(&pool, "ada@example.com", Role::Requester).await;
    let team: uuid::Uuid =
        sqlx::query_scalar("insert into teams (name) values ('Planning') returning id").fetch_one(&pool).await.unwrap();
    sqlx::query("update users set team_id = $1 where id = $2").bind(team).bind(user.id).execute(&pool).await.unwrap();

    let token = token_for(&pool, "ada@example.com").await;
    let (_, body) = me(&pool, &token).await;
    assert_eq!(body["teamId"], team.to_string());
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn provisioning_checks_its_input(pool: PgPool) {
    account(&pool, "ada@example.com", Role::Admin).await;
    let new = |email: &str, name: &str, password: &str| NewAccount {
        email: email.to_owned(),
        name: name.to_owned(),
        role: Role::Requester,
        password: password.to_owned(),
    };

    let taken = accounts::create(&pool, new("ADA@example.com", "Twin", PASSWORD)).await;
    assert!(matches!(taken, Err(AccountError::EmailTaken)), "{taken:?}");
    let short = accounts::create(&pool, new("bob@example.com", "Bob", "elevenchars")).await;
    assert!(matches!(short, Err(AccountError::ShortPassword)), "{short:?}");
    let email = accounts::create(&pool, new("bob@", "Bob", PASSWORD)).await;
    assert!(matches!(email, Err(AccountError::InvalidEmail)), "{email:?}");
    let name = accounts::create(&pool, new("bob@example.com", "  ", PASSWORD)).await;
    assert!(matches!(name, Err(AccountError::EmptyName)), "{name:?}");

    let bob = accounts::create(&pool, new("bob@example.com", "  Bob  ", PASSWORD)).await.unwrap();
    assert_eq!(bob.name, "Bob");
    assert!(bob.active);
    assert_eq!(bob.team_id, None);
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn the_service_provisions_operators(pool: PgPool) {
    account(&pool, "ops@example.com", Role::Operator).await;
    let token = token_for(&pool, "ops@example.com").await;
    assert_eq!(me(&pool, &token).await.1["role"], "operator");
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn a_password_reset_signs_the_account_out(pool: PgPool) {
    account(&pool, "ada@example.com", Role::Admin).await;
    let token = token_for(&pool, "ada@example.com").await;

    let reset = accounts::set_password(&pool, "ADA@example.com", "a brand new passphrase".to_owned()).await.unwrap();
    assert_eq!(reset.email, "ada@example.com");
    assert_eq!(me(&pool, &token).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(login(&pool, "ada@example.com", PASSWORD, "en").await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(login(&pool, "ada@example.com", "a brand new passphrase", "en").await.0, StatusCode::OK);

    let missing = accounts::set_password(&pool, "nobody@example.com", "a brand new passphrase".to_owned()).await;
    assert!(matches!(missing, Err(AccountError::NoSuchAccount)), "{missing:?}");
    let short = accounts::set_password(&pool, "ada@example.com", "short".to_owned()).await;
    assert!(matches!(short, Err(AccountError::ShortPassword)), "{short:?}");
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn the_schema_enforces_its_rules(pool: PgPool) {
    let role =
        sqlx::query("insert into users (email, name, role) values ('a@example.com', 'A', 'root')").execute(&pool).await;
    assert!(role.is_err(), "unknown role");

    sqlx::query("insert into teams (name) values ('Planning')").execute(&pool).await.unwrap();
    let too_long = "x".repeat(41);
    for name in ["planning", "", " Padded", too_long.as_str()] {
        let result = sqlx::query("insert into teams (name) values ($1)").bind(name).execute(&pool).await;
        assert!(result.is_err(), "team name {name:?}");
    }
    sqlx::query("insert into teams (name) values ($1)").bind("x".repeat(40)).execute(&pool).await.unwrap();

    let team: uuid::Uuid =
        sqlx::query_scalar("select id from teams where name = 'Planning'").fetch_one(&pool).await.unwrap();
    sqlx::query("insert into users (email, name, role, team_id) values ('b@example.com', 'B', 'admin', $1)")
        .bind(team)
        .execute(&pool)
        .await
        .unwrap();
    let delete = sqlx::query("delete from teams where id = $1").bind(team).execute(&pool).await;
    assert!(delete.is_err(), "a team with members cannot be deleted");

    // Migrating an up-to-date database is a no-op.
    db::migrate(&pool).await.unwrap();
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn password_guessing_is_locked_out(pool: PgPool) {
    account(&pool, "ada@example.com", Role::Developer).await;
    let wrong =
        (StatusCode::UNAUTHORIZED, json!({ "code": "unauthorized", "message": "The email or password is incorrect." }));
    let locked = (
        StatusCode::UNAUTHORIZED,
        json!({ "code": "unauthorized", "message": "Too many sign-in attempts. Try again in 15 minutes." }),
    );

    // A success clears the count: two misses, a sign-in, then five more misses are allowed.
    for _ in 0..2 {
        assert_eq!(login(&pool, "ada@example.com", "guess", "en").await, wrong);
    }
    assert_eq!(login(&pool, "ADA@example.com", PASSWORD, "en").await.0, StatusCode::OK);
    for _ in 0..5 {
        assert_eq!(login(&pool, "ada@example.com", "guess", "en").await, wrong);
    }
    // The sixth attempt is refused, even with the right password, in any case of the email.
    assert_eq!(login(&pool, "Ada@Example.com", PASSWORD, "en").await, locked);
    let (_, body) = login(&pool, "ada@example.com", PASSWORD, "ko").await;
    assert_eq!(body["message"], "로그인 시도가 너무 많습니다. 15분 뒤에 다시 시도해 주세요.");

    // An unknown email is counted and refused the same way.
    for _ in 0..5 {
        assert_eq!(login(&pool, "nobody@example.com", PASSWORD, "en").await, wrong);
    }
    assert_eq!(login(&pool, "nobody@example.com", PASSWORD, "en").await, locked);

    // Once the lockout has passed, the right password signs in again.
    sqlx::query("update login_attempts set locked_until = now() - interval '1 second'").execute(&pool).await.unwrap();
    assert_eq!(login(&pool, "ada@example.com", PASSWORD, "en").await.0, StatusCode::OK);
    let left: i64 = sqlx::query_scalar("select count(*) from login_attempts").fetch_one(&pool).await.unwrap();
    assert_eq!(left, 0, "signing in cleared the email's row and the sweep the unknown one's");
}

#[sqlx::test(migrator = "ax_api::db::MIGRATOR")]
async fn a_lock_outlasts_the_window_it_started_in(pool: PgPool) {
    account(&pool, "ada@example.com", Role::Developer).await;
    for _ in 0..6 {
        login(&pool, "ada@example.com", "guess", "en").await;
    }
    // The first miss is now older than the window, but the lock still holds.
    sqlx::query("update login_attempts set first_at = now() - interval '20 minutes'").execute(&pool).await.unwrap();
    let (status, body) = login(&pool, "ada@example.com", PASSWORD, "en").await;
    assert_eq!(
        (status, body["message"].as_str().map(|m| m.starts_with("Too many"))),
        (StatusCode::UNAUTHORIZED, Some(true))
    );
}
