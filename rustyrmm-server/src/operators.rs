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

/// Creates an Operator with an argon2id password hash and returns its id.
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
    sqlx::query("INSERT INTO operators (id, username, password_hash) VALUES ($1, $2, $3)")
        .bind(id)
        .bind(username)
        .bind(password_hash)
        .execute(pool)
        .await
        .map_err(|e| match e {
            sqlx::Error::Database(db) if db.is_unique_violation() => {
                AddOperatorError::UsernameTaken(username.to_owned())
            }
            e => AddOperatorError::Db(e),
        })?;
    Ok(id)
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
}
