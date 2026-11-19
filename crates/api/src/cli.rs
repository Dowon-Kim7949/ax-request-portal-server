//! Command line: `ax-api` serves; subcommands migrate and provision accounts.
//!
//! ```text
//! ax-api                                   # serve (same as `ax-api serve`)
//! ax-api migrate
//! ax-api user create --email E --name N --role admin [--password-stdin]
//! ax-api user set-password --email E [--password-stdin]
//! ax-api runner                            # take AI jobs from a server (see `runner`)
//! ```
//!
//! Passwords never go on the command line, where shell history and process
//! lists would keep them: they are typed at a prompt, or piped in with
//! `--password-stdin` (first line, without the line break).

use std::io::{self, BufRead, IsTerminal};

use clap::{Parser, Subcommand};
use sqlx::PgPool;

use crate::accounts::{self, NewAccount};
use crate::roles::Role;

#[derive(Debug, Parser)]
#[command(name = "ax-api", version, about = "AX Request Portal API server")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Command>,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Run the HTTP server (the default). Applies pending migrations first.
    Serve,
    /// Apply pending database migrations and exit.
    Migrate,
    /// Provision accounts. Operators can only be made here, never through the API.
    #[command(subcommand)]
    User(UserCommand),
    /// Take AI jobs from a server and run them with Claude Code on a clone of
    /// the project (settings: `AX_RUNNER_*`, `AX_GITHUB_*`, `AX_ANTHROPIC_*`).
    Runner,
}

#[derive(Debug, Subcommand)]
pub enum UserCommand {
    /// Create an active account with a password and no team.
    Create {
        #[arg(long)]
        email: String,
        #[arg(long)]
        name: String,
        /// requester, executive, developer, admin or operator
        #[arg(long, value_parser = parse_role)]
        role: Role,
        /// Read the password from the first line of standard input instead of prompting.
        #[arg(long)]
        password_stdin: bool,
    },
    /// Set or reset an account's password. Signs the account out everywhere.
    SetPassword {
        #[arg(long)]
        email: String,
        /// Read the password from the first line of standard input instead of prompting.
        #[arg(long)]
        password_stdin: bool,
    },
}

fn parse_role(value: &str) -> Result<Role, String> {
    Role::parse(value).ok_or_else(|| {
        let roles: Vec<&str> = Role::ALL.iter().map(|r| r.as_str()).collect();
        format!("expected one of {}", roles.join(", "))
    })
}

/// Runs a `user` subcommand and prints what it did.
///
/// # Errors
///
/// Reading the password, validation and database errors.
pub async fn run_user(db: &PgPool, command: UserCommand) -> Result<(), Box<dyn std::error::Error>> {
    match command {
        UserCommand::Create { email, name, role, password_stdin } => {
            let password = read_password(password_stdin)?;
            let user = accounts::create(db, NewAccount { email, name, role, password }).await?;
            println!("created {} {} ({})", user.role.as_str(), user.email, user.id);
        }
        UserCommand::SetPassword { email, password_stdin } => {
            let password = read_password(password_stdin)?;
            let user = accounts::set_password(db, &email, password).await?;
            println!("password set for {} ({}); their sessions were ended", user.email, user.id);
        }
    }
    Ok(())
}

fn read_password(from_stdin: bool) -> io::Result<String> {
    if from_stdin {
        let mut line = String::new();
        io::stdin().lock().read_line(&mut line)?;
        return Ok(line.trim_end_matches(['\r', '\n']).to_owned());
    }
    if !io::stdin().is_terminal() {
        return Err(io::Error::other("no terminal to prompt on; pipe the password in with --password-stdin"));
    }
    let password = rpassword::prompt_password("Password: ")?;
    if rpassword::prompt_password("Repeat password: ")? != password {
        return Err(io::Error::other("the passwords do not match"));
    }
    Ok(password)
}

#[cfg(test)]
mod tests {
    use clap::CommandFactory;

    use super::*;

    #[test]
    fn the_cli_is_well_formed() {
        Cli::command().debug_assert();
    }

    #[test]
    fn parses_user_create() {
        let cli = Cli::try_parse_from([
            "ax-api",
            "user",
            "create",
            "--email",
            "ada@example.com",
            "--name",
            "Ada",
            "--role",
            "operator",
            "--password-stdin",
        ])
        .unwrap();
        let Some(Command::User(UserCommand::Create { role, password_stdin, .. })) = cli.command else {
            panic!("parsed {cli:?}");
        };
        assert_eq!(role, Role::Operator);
        assert!(password_stdin);
    }

    #[test]
    fn rejects_unknown_roles_and_password_arguments() {
        let base = ["ax-api", "user", "create", "--email", "a@example.com", "--name", "A"];
        assert!(Cli::try_parse_from(base.iter().chain(&["--role", "root"])).is_err());
        assert!(Cli::try_parse_from(base.iter().chain(&["--role", "admin", "--password", "x"])).is_err());
    }

    #[test]
    fn no_subcommand_means_serve() {
        assert!(Cli::try_parse_from(["ax-api"]).unwrap().command.is_none());
    }
}
