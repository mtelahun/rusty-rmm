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
    #[error("could not hash password: {0}")]
    Hash(String),
    #[error(transparent)]
    Db(#[from] sqlx::Error),
}

/// Creates an Operator with an argon2id password hash and returns its id. The first Operator
/// becomes an Admin; later ones are not.
pub async fn add_operator(
    pool: &PgPool,
    username: &str,
    password: &str,
) -> Result<Uuid, AddOperatorError> {
    if username.is_empty() || username.chars().any(char::is_whitespace) {
        return Err(AddOperatorError::InvalidUsername);
    }
    if password.chars().count() < MIN_PASSWORD_LEN {
        return Err(AddOperatorError::PasswordTooShort);
    }
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
        "INSERT INTO operators (id, username, password_hash, is_admin)
         SELECT $1, $2, $3, NOT EXISTS (SELECT 1 FROM operators)",
    )
    .bind(id)
    .bind(username)
    .bind(password_hash)
    .execute(&mut *tx)
    .await
    .map_err(|e| match e {
        sqlx::Error::Database(db) if db.is_unique_violation() => {
            AddOperatorError::UsernameTaken(username.to_owned())
        }
        e => AddOperatorError::Db(e),
    })?;
    tx.commit().await?;
    Ok(id)
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
        let id = add_operator(&pool, "alice", GOOD_PASSWORD).await.unwrap();

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
        add_operator(&pool, "alice", GOOD_PASSWORD).await.unwrap();
        let err = add_operator(&pool, "alice", GOOD_PASSWORD)
            .await
            .unwrap_err();
        assert!(matches!(err, AddOperatorError::UsernameTaken(_)));
    }

    #[sqlx::test]
    async fn rejects_bad_input(pool: PgPool) {
        assert!(matches!(
            add_operator(&pool, "", GOOD_PASSWORD).await,
            Err(AddOperatorError::InvalidUsername)
        ));
        assert!(matches!(
            add_operator(&pool, "bob smith", GOOD_PASSWORD).await,
            Err(AddOperatorError::InvalidUsername)
        ));
        assert!(matches!(
            add_operator(&pool, "bob", "short").await,
            Err(AddOperatorError::PasswordTooShort)
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
        add_operator(&pool, "alice", GOOD_PASSWORD).await.unwrap();
        add_operator(&pool, "bob", GOOD_PASSWORD).await.unwrap();
        assert!(is_admin(&pool, "alice").await);
        assert!(!is_admin(&pool, "bob").await);
    }

    #[sqlx::test]
    async fn the_last_admin_cannot_be_removed(pool: PgPool) {
        add_operator(&pool, "alice", GOOD_PASSWORD).await.unwrap();
        add_operator(&pool, "bob", GOOD_PASSWORD).await.unwrap();
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
