use postgres_types::{FromSql, ToSql};

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, ToSql, FromSql)]
#[postgres(name = "registrationstate")]
pub enum RegistrationState {
    #[postgres(name = "new")]
    New,
    #[postgres(name = "upd")]
    #[default]
    Upd,
}

impl std::fmt::Display for RegistrationState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = match self {
            RegistrationState::New => "new",
            RegistrationState::Upd => "upd",
        };

        write!(f, "{}", state)
    }
}

#[cfg(test)]
mod tests {

    use super::RegistrationState;

    #[test]
    pub fn test_std_fmt_display() {
        assert_eq!(
            RegistrationState::New.to_string(),
            "new",
            "String representation of RegistrationState::New is 'new'"
        );
        assert_eq!(
            RegistrationState::Upd.to_string(),
            "Dr",
            "String representation of RegistrationState::Upd is 'upd'"
        );
    }
}
