use argon2::password_hash::PasswordHasher;
use argon2::Argon2;
use sqlx::PgPool;
use uuid::Uuid;

pub const MIN_PASSWORD_LEN: usize = 12;

#[derive(Debug, thiserror::Error)]
pub enum AddOperatorError {
    #[error("username must not be empty or contain whitespace")]
    InvalidUsername,
    #[error("password must be at least {MIN_PASSWORD_LEN} characters")]
    PasswordTooShort,
    #[error("an Operator named {0:?} already exists")]
    UsernameTaken(String),
    #[error("email address is required")]
    EmptyEmail,
    #[error("email address is not valid")]
    InvalidEmail,
    #[error("an Operator with email {0:?} already exists")]
    EmailTaken(String),
    #[error("could not hash password: {0}")]
    Hash(String),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum EmailError {
    #[error("email address is required")]
    Empty,
    #[error("email address is not valid")]
    Invalid,
}

/// Trims an email address and checks its shape: one `@`, a local part, a domain containing a
/// `.`, and no whitespace. Format only; nothing is verified.
pub fn validate_email(email: &str) -> Result<&str, EmailError> {
    let email = email.trim();
    if email.is_empty() {
        return Err(EmailError::Empty);
    }
    let ok = !email.chars().any(char::is_whitespace)
        && email.split_once('@').is_some_and(|(local, domain)| {
            !local.is_empty() && !domain.contains('@') && domain.contains('.')
        });
    if ok {
        Ok(email)
    } else {
        Err(EmailError::Invalid)
    }
}

impl From<EmailError> for AddOperatorError {
    fn from(e: EmailError) -> Self {
        match e {
            EmailError::Empty => Self::EmptyEmail,
            EmailError::Invalid => Self::InvalidEmail,
        }
    }
}

/// Creates an Operator with an argon2id password hash and returns its id. The first Operator
/// becomes an Admin; later ones are not.
pub async fn add_operator(
    pool: &PgPool,
    username: &str,
    password: &str,
    email: &str,
) -> Result<Uuid, AddOperatorError> {
    if username.is_empty() || username.chars().any(char::is_whitespace) {
        return Err(AddOperatorError::InvalidUsername);
    }
    if password.chars().count() < MIN_PASSWORD_LEN {
        return Err(AddOperatorError::PasswordTooShort);
    }
    let email = validate_email(email)?;
    let password_hash = Argon2::default()
        .hash_password(password.as_bytes())
        .map_err(|e| AddOperatorError::Hash(e.to_string()))?
        .to_string();

    let id = Uuid::now_v7();
    let mut tx = pool.begin().await?;
    // Blocks other writers so two first Operators can't both see an empty table.
    sqlx::query("LOCK TABLE operators IN EXCLUSIVE MODE")
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO operators (id, username, password_hash, email, is_admin)
         SELECT $1, $2, $3, $4, NOT EXISTS (SELECT 1 FROM operators)",
    )
    .bind(id)
    .bind(username)
    .bind(password_hash)
    .bind(email)
    .execute(&mut *tx)
    .await
    .map_err(|e| match e {
        sqlx::Error::Database(db) if db.constraint() == Some("operators_email_lower") => {
            AddOperatorError::EmailTaken(email.to_owned())
        }
        sqlx::Error::Database(db) if db.is_unique_violation() => {
            AddOperatorError::UsernameTaken(username.to_owned())
        }
        e => AddOperatorError::Db(e),
    })?;
    tx.commit().await?;
    Ok(id)
}

#[derive(Debug, thiserror::Error)]
pub enum SetEmailError {
    #[error(transparent)]
    Invalid(#[from] EmailError),
    #[error("an Operator with email {0:?} already exists")]
    EmailTaken(String),
    #[error("no Operator named {0:?}")]
    NoSuchOperator(String),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

/// Changes an Operator's email address.
pub async fn set_email(pool: &PgPool, username: &str, email: &str) -> Result<(), SetEmailError> {
    let email = validate_email(email)?;
    let found = sqlx::query("UPDATE operators SET email = $2 WHERE username = $1")
        .bind(username)
        .bind(email)
        .execute(pool)
        .await
        .map_err(|e| match e {
            sqlx::Error::Database(db) if db.is_unique_violation() => {
                SetEmailError::EmailTaken(email.to_owned())
            }
            e => SetEmailError::Db(e),
        })?
        .rows_affected();
    if found == 0 {
        return Err(SetEmailError::NoSuchOperator(username.to_owned()));
    }
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum SetAdminError {
    #[error("no Operator named {0:?}")]
    NoSuchOperator(String),
    #[error("at least one Operator must remain an Admin")]
    LastAdmin,
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

/// Adds or removes an Operator from the Admin group. The last Admin can't be removed.
pub async fn set_admin(pool: &PgPool, username: &str, is_admin: bool) -> Result<(), SetAdminError> {
    let mut tx = pool.begin().await?;
    // Locking every Admin row makes concurrent removals take turns, so each sees the other's result.
    let admins: Vec<String> =
        sqlx::query_scalar("SELECT username FROM operators WHERE is_admin ORDER BY id FOR UPDATE")
            .fetch_all(&mut *tx)
            .await?;
    if !is_admin && admins.len() == 1 && admins[0] == username {
        return Err(SetAdminError::LastAdmin);
    }
    let found = sqlx::query("UPDATE operators SET is_admin = $2 WHERE username = $1")
        .bind(username)
        .bind(is_admin)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if found == 0 {
        return Err(SetAdminError::NoSuchOperator(username.to_owned()));
    }
    tx.commit().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use argon2::password_hash::{phc::PasswordHash, PasswordVerifier};

    const GOOD_PASSWORD: &str = "correct horse battery";

    #[sqlx::test]
    async fn stores_a_verifiable_argon2_hash(pool: PgPool) {
        let id = add_operator(&pool, "alice", GOOD_PASSWORD, "alice@example.com")
            .await
            .unwrap();

        let stored: String =
            sqlx::query_scalar("SELECT password_hash FROM operators WHERE id = $1")
                .bind(id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(stored.starts_with("$argon2id$"));
        let parsed = PasswordHash::new(&stored).unwrap();
        assert!(Argon2::default()
            .verify_password(GOOD_PASSWORD.as_bytes(), &parsed)
            .is_ok());
        assert_eq!(id.get_version_num(), 7);
    }

    #[sqlx::test]
    async fn rejects_duplicate_username(pool: PgPool) {
        add_operator(&pool, "alice", GOOD_PASSWORD, "alice@example.com")
            .await
            .unwrap();
        let err = add_operator(&pool, "alice", GOOD_PASSWORD, "alice@example.com")
            .await
            .unwrap_err();
        assert!(matches!(err, AddOperatorError::UsernameTaken(_)));
    }

    #[sqlx::test]
    async fn rejects_bad_input(pool: PgPool) {
        assert!(matches!(
            add_operator(&pool, "", GOOD_PASSWORD, "a@b.co").await,
            Err(AddOperatorError::InvalidUsername)
        ));
        assert!(matches!(
            add_operator(&pool, "bob smith", GOOD_PASSWORD, "a@b.co").await,
            Err(AddOperatorError::InvalidUsername)
        ));
        assert!(matches!(
            add_operator(&pool, "bob", "short", "a@b.co").await,
            Err(AddOperatorError::PasswordTooShort)
        ));
    }

    #[sqlx::test]
    async fn rejects_bad_email(pool: PgPool) {
        for email in [
            "",
            "  ",
            "x",
            "a@b",
            "a b@c.d",
            "@c.d",
            "a@@c.d",
            "a@c.d@e.f",
        ] {
            let err = add_operator(&pool, "bob", GOOD_PASSWORD, email)
                .await
                .unwrap_err();
            assert!(
                matches!(
                    err,
                    AddOperatorError::EmptyEmail | AddOperatorError::InvalidEmail
                ),
                "{email:?}"
            );
        }
    }

    async fn email_of(pool: &PgPool, username: &str) -> Option<String> {
        sqlx::query_scalar("SELECT email FROM operators WHERE username = $1")
            .bind(username)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    #[sqlx::test]
    async fn email_is_unique_ignoring_case(pool: PgPool) {
        add_operator(&pool, "alice", GOOD_PASSWORD, " Alice@Example.com ")
            .await
            .unwrap();
        assert_eq!(
            email_of(&pool, "alice").await.as_deref(),
            Some("Alice@Example.com")
        );
        let err = add_operator(&pool, "bob", GOOD_PASSWORD, "alice@example.COM")
            .await
            .unwrap_err();
        assert!(matches!(err, AddOperatorError::EmailTaken(_)));
        add_operator(&pool, "bob", GOOD_PASSWORD, "bob@example.com")
            .await
            .unwrap();
        let err = set_email(&pool, "bob", "ALICE@example.com")
            .await
            .unwrap_err();
        assert!(matches!(err, SetEmailError::EmailTaken(_)));
        set_email(&pool, "bob", "bob2@example.com").await.unwrap();
        assert!(matches!(
            set_email(&pool, "nobody", "n@example.com").await,
            Err(SetEmailError::NoSuchOperator(_))
        ));
    }

    async fn is_admin(pool: &PgPool, username: &str) -> bool {
        sqlx::query_scalar("SELECT is_admin FROM operators WHERE username = $1")
            .bind(username)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    #[sqlx::test]
    async fn only_the_first_operator_is_admin(pool: PgPool) {
        add_operator(&pool, "alice", GOOD_PASSWORD, "alice@example.com")
            .await
            .unwrap();
        add_operator(&pool, "bob", GOOD_PASSWORD, "bob@example.com")
            .await
            .unwrap();
        assert!(is_admin(&pool, "alice").await);
        assert!(!is_admin(&pool, "bob").await);
    }

    #[sqlx::test]
    async fn the_last_admin_cannot_be_removed(pool: PgPool) {
        add_operator(&pool, "alice", GOOD_PASSWORD, "alice@example.com")
            .await
            .unwrap();
        add_operator(&pool, "bob", GOOD_PASSWORD, "bob@example.com")
            .await
            .unwrap();
        assert!(matches!(
            set_admin(&pool, "alice", false).await,
            Err(SetAdminError::LastAdmin)
        ));
        assert!(is_admin(&pool, "alice").await);

        set_admin(&pool, "bob", true).await.unwrap();
        set_admin(&pool, "alice", false).await.unwrap();
        assert!(!is_admin(&pool, "alice").await);
        assert!(matches!(
            set_admin(&pool, "nobody", true).await,
            Err(SetAdminError::NoSuchOperator(_))
        ));
    }

    #[sqlx::test(migrations = false)]
    async fn migration_0004_keeps_operators_with_null_email(pool: PgPool) {
        for sql in [
            include_str!("../migrations/0001_initial.sql"),
            include_str!("../migrations/0002_command_timed_out.sql"),
            include_str!("../migrations/0003_admin_and_environment_members.sql"),
        ] {
            sqlx::raw_sql(sql).execute(&pool).await.unwrap();
        }
        sqlx::raw_sql(
            "INSERT INTO operators (id, username, password_hash) VALUES
               ('00000000-0000-0000-0000-000000000001', 'a', 'x'),
               ('00000000-0000-0000-0000-000000000002', 'b', 'x');",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::raw_sql(include_str!("../migrations/0004_operator_email.sql"))
            .execute(&pool)
            .await
            .unwrap();
        let nulls: i64 = sqlx::query_scalar("SELECT count(*) FROM operators WHERE email IS NULL")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(nulls, 2);
    }

    #[sqlx::test(migrations = false)]
    async fn migration_0003_backfills_admins_and_memberships(pool: PgPool) {
        for sql in [
            include_str!("../migrations/0001_initial.sql"),
            include_str!("../migrations/0002_command_timed_out.sql"),
        ] {
            sqlx::raw_sql(sql).execute(&pool).await.unwrap();
        }
        sqlx::raw_sql(
            "INSERT INTO operators (id, username, password_hash) VALUES
               ('00000000-0000-0000-0000-000000000001', 'a', 'x'),
               ('00000000-0000-0000-0000-000000000002', 'b', 'x');
             INSERT INTO environments (id, name) VALUES
               ('00000000-0000-0000-0000-0000000000e1', 'one'),
               ('00000000-0000-0000-0000-0000000000e2', 'two');",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::raw_sql(include_str!(
            "../migrations/0003_admin_and_environment_members.sql"
        ))
        .execute(&pool)
        .await
        .unwrap();

        let admins: i64 = sqlx::query_scalar("SELECT count(*) FROM operators WHERE is_admin")
            .fetch_one(&pool)
            .await
            .unwrap();
        let members: i64 = sqlx::query_scalar("SELECT count(*) FROM environment_members")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!((admins, members), (2, 4));
    }
}
