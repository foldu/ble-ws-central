use directories_next::ProjectDirs;
use eyre::Context;
use once_cell::sync::Lazy;
use rand::{distributions, prelude::*, rngs::OsRng};
use std::{
    fs,
    io::prelude::*,
    net::{IpAddr, Ipv4Addr},
    num::NonZeroU8,
    os::unix::prelude::*,
    path::PathBuf,
};
use tokio_mqtt as mqtt;

#[derive(serde::Deserialize)]
struct EnvConfig {
    mqtt_server_url: Option<url::Url>,
    mqtt_cert_file: Option<PathBuf>,
    #[serde(default = "tru")]
    enable_mesh: bool,
    #[serde(default = "default_host")]
    pub host: IpAddr,
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_db_path")]
    pub db_path: PathBuf,
    pub demo: Option<NonZeroU8>,
    pub token: Option<String>,
}

fn tru() -> bool {
    true
}

fn default_host() -> IpAddr {
    Ipv4Addr::LOCALHOST.into()
}

fn default_port() -> u16 {
    8080
}

static DATA_DIR: Lazy<PathBuf> = Lazy::new(|| {
    ProjectDirs::from("org", "foldu", env!("CARGO_PKG_NAME"))
        .ok_or_else(|| eyre::format_err!("Could not get project directories"))
        .unwrap()
        .data_dir()
        .to_owned()
});

fn default_db_path() -> PathBuf {
    DATA_DIR.join(concat!(env!("CARGO_PKG_NAME"), ".mdb"))
}

pub(crate) struct Config {
    pub mqtt_options: Option<mqtt::ConnectOptions>,
    pub enable_mesh: bool,
    pub host: IpAddr,
    pub port: u16,
    pub db_path: PathBuf,
    pub demo: Option<NonZeroU8>,
    pub token: tonic::metadata::MetadataValue<tonic::metadata::Ascii>,
}

/// type used to prevent showing the token in the server logs
pub struct Secret(String);

impl Secret {
    pub fn check(&self, s: &str) -> bool {
        // FIXME: BUG: non constant time comparison
        &self.0 == s
    }
}

impl Config {
    pub fn from_env() -> Result<Self, eyre::Error> {
        let env_config: EnvConfig =
            envy::from_env().context("Could not read config from environment")?;
        let mqtt_options = env_config.mqtt_server_url.as_ref().map(|url| -> Result<_, eyre::Error> {
            let ssl = if url.scheme() == "mqtts" {
                let cert_path = env_config
                    .mqtt_cert_file
                    .as_ref()
                    .ok_or_else(|| eyre::format_err!("Need a cert file for mqtts url but environment variable MQTT_CERT_FILE was not set"))?;
                let pem = fs::read(&cert_path)
                    .with_context(|| eyre::format_err!("Could not read cert pem from {}", cert_path.display()))?;
                mqtt::Ssl::WithCert(pem)
            } else {
                mqtt::Ssl::None
            };
            mqtt::ConnectOptions::new(&url, ssl).map_err(|e| e.into())
        }).transpose()?;

        let token_path = DATA_DIR.join("token");
        let token = match env_config.token {
            Some(token) => token,
            None => match std::fs::read_to_string(&token_path) {
                Ok(token) => {
                    let token = token.trim();
                    if token.is_empty() {
                        eyre::bail!(
                            "Token in {} is empty, refusing to proceed",
                            token_path.display()
                        );
                    }
                    token.to_string()
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    if let Some(parent) = token_path.parent() {
                        let _ = std::fs::create_dir_all(&parent);
                    }

                    let token = OsRng
                        .sample_iter(distributions::Alphanumeric)
                        .take(64)
                        .map(char::from)
                        .collect::<String>();
                    let mut fh = std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .mode(0o600)
                        .open(&token_path)
                        .with_context(|| {
                            format!("Could not open token file {}", token_path.display())
                        })?;
                    fh.write(token.as_bytes())
                        .and_then(|_| fh.flush())
                        .with_context(|| {
                            format!("Could not write token file {}", token_path.display())
                        })?;
                    token
                }
                Err(e) => {
                    eyre::bail!("Could not read token in {}: {}", token_path.display(), e);
                }
            },
        };

        let mut token = tonic::metadata::MetadataValue::from_str(&token)
            .map_err(|_| eyre::format_err!("Corrupted token file {}", token_path.display()))?;
        token.set_sensitive(true);

        Ok(Self {
            mqtt_options,
            enable_mesh: env_config.enable_mesh,
            host: env_config.host,
            port: env_config.port,
            db_path: env_config.db_path,
            demo: env_config.demo,
            token,
        })
    }
}
