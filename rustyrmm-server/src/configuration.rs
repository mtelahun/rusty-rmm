use std::path::PathBuf;

use secrecy::{ExposeSecret, Secret};
use serde::Deserialize;

use crate::APP_NAME;

#[derive(Deserialize)]
pub struct Settings {
    pub database: DatabaseSettings,
    pub listen_address: String,
    pub listen_port: u16,
}

#[derive(Deserialize)]
pub struct DatabaseSettings {
    pub username: String,
    pub password: Secret<String>,
    pub host: String,
    pub port: u16,
    pub database_name: String,
    pub database_url: Option<String>,
}

pub fn get_configuration(
    config_file_path: Option<PathBuf>,
    no_config: bool,
) -> Result<Settings, config::ConfigError> {
    let mut settings = config::Config::builder()
        .set_default("listen_address", "127.0.0.1")?
        .set_default("listen_port", 50052)?
        .set_default("database.host", "127.0.0.1")?
        .set_default("database.port", 5432)?
        .set_default("database.username", "postgres")?
        .set_default("database.password", "password")?
        .set_default("database.database_name", "rustyrmm-server")?;
    if let Some(path) = config_file_path {
        if !no_config {
            settings = settings.add_source(config::File::from(path));
        }
    } else {
        settings = settings.add_source(config::Environment::with_prefix(APP_NAME));
    }
    let settings = settings.build()?;

    settings.try_deserialize()
}

impl DatabaseSettings {
    pub fn database_url(&self) -> String {
        if let Some(url) = self.database_url.clone() {
            url
        } else {
            format!(
                "postgres://{}:{}@{}:{}/{}",
                self.username,
                self.password.expose_secret(),
                self.host,
                self.port,
                self.database_name
            )
        }
    }

    pub fn postgres_url(&self) -> String {
        if let Some(url) = self.database_url.clone() {
            let split = url.rsplit_once('/').unwrap();

            String::from(split.0)
        } else {
            format!(
                "postgres://{}:{}@{}:{}",
                self.username,
                self.password.expose_secret(),
                self.host,
                self.port,
            )
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn default_settings() {
        let settings =
            get_configuration(None, false).expect("failed to get configuration settings");

        assert_eq!(settings.listen_address, "127.0.0.1", "Listen on localhost");
        assert_eq!(settings.listen_port, 50052, "Listen on port 50052");
        assert_eq!(
            settings.database.database_name, "rustyrmm-server",
            "database name is rustyrmm-server"
        );
        assert_eq!(
            settings.database.host, "127.0.0.1",
            "database host is localhost"
        );
        assert_eq!(settings.database.port, 5432, "database port is 5432");
        assert_eq!(
            settings.database.username, "postgres",
            "database user is postgres"
        );
        assert_eq!(
            settings.database.password.expose_secret(),
            "password",
            "database password matches"
        );
        assert_eq!(
            settings.database.database_url(),
            "postgres://postgres:password@127.0.0.1:5432/rustyrmm-server",
            "database url is correct"
        );
        assert_eq!(
            settings.database.postgres_url(),
            "postgres://postgres:password@127.0.0.1:5432",
            "database url is correct"
        );
    }

    #[test]
    fn database_url_from_database_url_setting() {
        let mut settings =
            get_configuration(None, false).expect("failed to get configuration settings");
        settings.database.database_url = Some(String::from(
            "postgres://postgres:123456@127.0.0.1:5432/rustyrmm-server",
        ));

        assert_eq!(
            settings.database.database_url(),
            "postgres://postgres:123456@127.0.0.1:5432/rustyrmm-server",
            "database url is correct"
        );

        assert_eq!(
            settings.database.postgres_url(),
            "postgres://postgres:123456@127.0.0.1:5432",
            "postgres url is correct"
        );
    }
}
