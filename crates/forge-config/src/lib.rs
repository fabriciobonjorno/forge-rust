//! Typed runtime configuration with explicit defaults and validation.
//!
//! Environment access is isolated at the process boundary. Application code
//! receives a validated [`AppConfig`] and never needs to parse ad-hoc strings.

use std::{
    collections::HashMap,
    ffi::OsString,
    fmt,
    net::{IpAddr, SocketAddr},
    str::FromStr,
    time::Duration,
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

const DEFAULT_BIND: &str = "127.0.0.1:3000";
const DEFAULT_REQUEST_TIMEOUT_SECS: u64 = 30;
const DEFAULT_SHUTDOWN_GRACE_SECS: u64 = 15;
const DEFAULT_MAX_BODY_BYTES: usize = 1_048_576;
const DEFAULT_MAX_CONNECTIONS: usize = 10_000;
const DEFAULT_LOG_FILTER: &str = "info";

/// The database scheme (protocol) used in a connection URL.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DatabaseScheme {
    /// PostgreSQL database (`postgres://` or `postgresql://`).
    #[default]
    Postgres,
    /// MySQL or MariaDB database (`mysql://` or `mariadb://`).
    MySql,
    /// SQLite database (`sqlite://`).
    Sqlite,
}

impl FromStr for DatabaseScheme {
    type Err = ConfigError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "postgres" | "postgresql" => Ok(Self::Postgres),
            "mysql" | "mariadb" => Ok(Self::MySql),
            "sqlite" => Ok(Self::Sqlite),
            _ => Err(ConfigError::InvalidValue {
                key: "FORGE_DATABASE_URL",
                value: value.to_owned(),
                expected: "postgres, mysql, or sqlite",
            }),
        }
    }
}

impl fmt::Display for DatabaseScheme {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Postgres => write!(f, "postgres"),
            Self::MySql => write!(f, "mysql"),
            Self::Sqlite => write!(f, "sqlite"),
        }
    }
}

/// Prefix reserved for Forge configuration keys.
///
/// Every environment variable starting with this prefix must be one of
/// [`KNOWN_KEYS`]; anything else is rejected by [`AppConfig::from_env`].
const KEY_PREFIX: &str = "FORGE_";

/// Every environment variable understood by [`AppConfig::from_env`].
pub const KNOWN_KEYS: &[&str] = &[
    "FORGE_ENV",
    "FORGE_BIND",
    "FORGE_REQUEST_TIMEOUT_SECS",
    "FORGE_SHUTDOWN_GRACE_SECS",
    "FORGE_MAX_BODY_BYTES",
    "FORGE_MAX_CONNECTIONS",
    "FORGE_PUBLIC_ORIGIN",
    "FORGE_TRUSTED_PROXIES",
    "FORGE_LOG",
    "FORGE_LOG_FORMAT",
    "FORGE_DATABASE_URL",
    "FORGE_MIGRATION_DATABASE_URL",
];

/// Runtime environment controls safety-sensitive defaults.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Environment {
    /// Local developer workstation.
    #[default]
    Development,
    /// Automated test process.
    Test,
    /// Internet-facing production process.
    Production,
}

impl FromStr for Environment {
    type Err = ConfigError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "development" | "dev" => Ok(Self::Development),
            "test" => Ok(Self::Test),
            "production" | "prod" => Ok(Self::Production),
            _ => Err(ConfigError::InvalidValue {
                key: "FORGE_ENV",
                value: value.to_owned(),
                expected: "development, test, or production",
            }),
        }
    }
}

/// HTTP server settings after validation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Address on which the process accepts connections.
    pub bind: SocketAddr,
    /// Maximum time allowed for one request.
    #[serde(with = "duration_seconds")]
    pub request_timeout: Duration,
    /// Time allowed for in-flight work during shutdown.
    #[serde(with = "duration_seconds")]
    pub shutdown_grace: Duration,
    /// Maximum buffered request body size.
    pub max_body_bytes: usize,
    /// Maximum number of concurrently open client connections.
    pub max_connections: usize,
    /// Canonical public origin used for browser Origin/CSRF validation.
    /// Empty by default; authentication routes must fail closed when absent.
    #[serde(default)]
    pub public_origin: Option<String>,
    /// Explicit proxy CIDRs allowed to supply `X-Forwarded-For` chains.
    /// Empty by default: all forwarding headers are ignored.
    #[serde(default)]
    pub trusted_proxies: Vec<IpNetwork>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind: DEFAULT_BIND
                .parse()
                .unwrap_or_else(|error| unreachable!("valid built-in bind address: {error}")),
            request_timeout: Duration::from_secs(DEFAULT_REQUEST_TIMEOUT_SECS),
            shutdown_grace: Duration::from_secs(DEFAULT_SHUTDOWN_GRACE_SECS),
            max_body_bytes: DEFAULT_MAX_BODY_BYTES,
            max_connections: DEFAULT_MAX_CONNECTIONS,
            public_origin: None,
            trusted_proxies: Vec::new(),
        }
    }
}

/// IPv4 or IPv6 network used to define trusted reverse proxies.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IpNetwork {
    address: IpAddr,
    prefix: u8,
}

impl IpNetwork {
    /// Returns whether `address` falls within this network.
    #[must_use]
    pub fn contains(self, address: IpAddr) -> bool {
        match (self.address, address) {
            (IpAddr::V4(network), IpAddr::V4(candidate)) => {
                prefix_matches_v4(u32::from(network), u32::from(candidate), self.prefix)
            }
            (IpAddr::V6(network), IpAddr::V6(candidate)) => {
                prefix_matches_v6(u128::from(network), u128::from(candidate), self.prefix)
            }
            (IpAddr::V4(network), IpAddr::V6(candidate)) => {
                candidate.to_ipv4_mapped().is_some_and(|candidate| {
                    prefix_matches_v4(u32::from(network), u32::from(candidate), self.prefix)
                })
            }
            _ => false,
        }
    }
}

impl fmt::Display for IpNetwork {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}/{}", self.address, self.prefix)
    }
}

impl Serialize for IpNetwork {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for IpNetwork {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse().map_err(serde::de::Error::custom)
    }
}

impl FromStr for IpNetwork {
    type Err = &'static str;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let mut parts = value.split('/');
        let address = parts
            .next()
            .ok_or("expected an IP address")?
            .parse::<IpAddr>()
            .map_err(|_| "expected an IPv4 or IPv6 address")?;
        let max_prefix = match address {
            IpAddr::V4(_) => 32,
            IpAddr::V6(_) => 128,
        };
        let prefix = match parts.next() {
            Some(value) => {
                let prefix = value
                    .parse::<u8>()
                    .map_err(|_| "expected a numeric network prefix")?;
                if prefix > max_prefix {
                    return Err("network prefix is too large for the address family");
                }
                prefix
            }
            None => max_prefix,
        };
        if parts.next().is_some() {
            return Err("expected at most one network prefix");
        }
        let address = match address {
            IpAddr::V4(address) => {
                let numeric = u32::from(address);
                let mask = mask_v4(prefix);
                IpAddr::V4(numeric_to_v4(numeric & mask))
            }
            IpAddr::V6(address) => {
                let numeric = u128::from(address);
                let mask = mask_v6(prefix);
                IpAddr::V6(numeric_to_v6(numeric & mask))
            }
        };
        Ok(Self { address, prefix })
    }
}

fn prefix_matches_v4(network: u32, candidate: u32, prefix: u8) -> bool {
    let mask = mask_v4(prefix);
    network & mask == candidate & mask
}

fn prefix_matches_v6(network: u128, candidate: u128, prefix: u8) -> bool {
    let mask = mask_v6(prefix);
    network & mask == candidate & mask
}

fn mask_v4(prefix: u8) -> u32 {
    if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - u32::from(prefix))
    }
}

fn mask_v6(prefix: u8) -> u128 {
    if prefix == 0 {
        0
    } else {
        u128::MAX << (128 - u32::from(prefix))
    }
}

fn numeric_to_v4(value: u32) -> std::net::Ipv4Addr {
    std::net::Ipv4Addr::from(value)
}

fn numeric_to_v6(value: u128) -> std::net::Ipv6Addr {
    std::net::Ipv6Addr::from(value)
}

/// Log output encoding.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    /// One JSON object per line, suitable for log collectors.
    Json,
    /// Human-readable text for local development.
    #[default]
    Text,
}

impl LogFormat {
    /// Default format for an environment: JSON in production, text elsewhere.
    #[must_use]
    pub fn default_for(environment: Environment) -> Self {
        match environment {
            Environment::Production => Self::Json,
            Environment::Development | Environment::Test => Self::Text,
        }
    }
}

impl FromStr for LogFormat {
    type Err = ConfigError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value.trim().to_ascii_lowercase().as_str() {
            "json" => Ok(Self::Json),
            "text" => Ok(Self::Text),
            _ => Err(ConfigError::InvalidValue {
                key: "FORGE_LOG_FORMAT",
                value: value.to_owned(),
                expected: "json or text",
            }),
        }
    }
}

/// Logging settings after validation.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LogConfig {
    /// Filter directives in `tracing-subscriber` `EnvFilter` syntax, e.g. `info,my_app=debug`.
    pub filter: String,
    /// Output encoding.
    pub format: LogFormat,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            filter: DEFAULT_LOG_FILTER.to_owned(),
            format: LogFormat::default_for(Environment::default()),
        }
    }
}

/// A secret configuration value.
///
/// Debug and serialization are deliberately redacted so credentials cannot be
/// exposed by ordinary diagnostics. Call expose only at the adapter boundary
/// that needs the underlying value.
#[derive(Clone, Eq, PartialEq, Deserialize)]
#[serde(transparent)]
pub struct SecretString(String);

impl SecretString {
    /// Wraps a secret value.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Exposes the secret to a concrete adapter.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Parses the secret as a URL and returns its scheme as a DatabaseScheme (e.g., postgres, mysql, sqlite).
    /// Returns None if the secret is not a valid URL or the scheme is unknown.
    pub fn scheme(&self) -> Option<DatabaseScheme> {
        url::Url::parse(&self.0)
            .ok()
            .and_then(|u| u.scheme().parse().ok())
    }
}

impl std::fmt::Debug for SecretString {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

impl Serialize for SecretString {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str("[REDACTED]")
    }
}

/// Database settings after validation.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct DatabaseConfig {
    /// Least-privilege PostgreSQL runtime URL. Optional for database-free apps.
    pub url: Option<SecretString>,
    /// Privileged PostgreSQL URL used only by migrate/rollback commands.
    pub migration_url: Option<SecretString>,
    /// Maximum number of connections in the pool.
    pub max_connections: Option<u32>,
    /// Timeout for acquiring a connection from the pool.
    #[serde(with = "duration_seconds_option")]
    pub acquire_timeout: Option<std::time::Duration>,
}

impl DatabaseConfig {
    /// Creates a new DatabaseConfig with just the URL set.
    pub fn new(url: SecretString) -> Self {
        Self {
            url: Some(url),
            migration_url: None,
            max_connections: None,
            acquire_timeout: None,
        }
    }

    /// Returns the database scheme from the URL if available.
    pub fn scheme(&self) -> Option<DatabaseScheme> {
        self.url.as_ref().and_then(|u| u.scheme())
    }
}

/// Complete process configuration.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct AppConfig {
    /// Deployment environment.
    pub environment: Environment,
    /// HTTP server settings.
    pub server: ServerConfig,
    /// Logging settings.
    pub log: LogConfig,
    /// Database settings.
    pub database: DatabaseConfig,
}

impl AppConfig {
    /// Loads configuration from process environment variables.
    ///
    /// Supported keys are listed in [`KNOWN_KEYS`]. Loading fails closed: any
    /// other variable starting with `FORGE_` is rejected with
    /// [`ConfigError::UnknownKey`], so a typo such as `FORGE_BIDN` cannot
    /// silently fall back to a default.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::from_vars(std::env::vars_os())
    }

    /// Loads configuration from an explicit set of environment variables.
    ///
    /// Variables without the `FORGE_` prefix are ignored. Unknown `FORGE_*`
    /// keys and known keys whose value is not valid UTF-8 are rejected.
    pub fn from_vars(
        vars: impl IntoIterator<Item = (OsString, OsString)>,
    ) -> Result<Self, ConfigError> {
        let mut values: HashMap<&'static str, String> = HashMap::new();
        for (key, value) in vars {
            let key_text = key.to_string_lossy();
            if !key_text.starts_with(KEY_PREFIX) {
                continue;
            }
            let Some(known) = KNOWN_KEYS
                .iter()
                .copied()
                .find(|known| **known == *key_text)
            else {
                return Err(ConfigError::UnknownKey {
                    key: key_text.into_owned(),
                });
            };
            let value = value
                .into_string()
                .map_err(|_| ConfigError::NotUnicode { key: known })?;
            values.insert(known, value);
        }
        Self::from_lookup(|key| values.get(key).cloned())
    }

    /// Loads configuration using a caller-provided environment lookup.
    ///
    /// Only known keys are looked up, so unknown-key detection is the
    /// responsibility of [`AppConfig::from_vars`]. This is useful for
    /// deterministic tests and configuration adapters.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, ConfigError> {
        let mut config = Self::default();

        if let Some(value) = lookup("FORGE_ENV") {
            config.environment = value.parse()?;
        }
        if let Some(value) = lookup("FORGE_BIND") {
            config.server.bind = parse_value("FORGE_BIND", &value, "a socket address")?;
        }
        if let Some(value) = lookup("FORGE_REQUEST_TIMEOUT_SECS") {
            config.server.request_timeout =
                Duration::from_secs(parse_positive("FORGE_REQUEST_TIMEOUT_SECS", &value)?);
        }
        if let Some(value) = lookup("FORGE_SHUTDOWN_GRACE_SECS") {
            config.server.shutdown_grace =
                Duration::from_secs(parse_positive("FORGE_SHUTDOWN_GRACE_SECS", &value)?);
        }
        if let Some(value) = lookup("FORGE_MAX_BODY_BYTES") {
            config.server.max_body_bytes = parse_positive("FORGE_MAX_BODY_BYTES", &value)?;
        }
        if let Some(value) = lookup("FORGE_MAX_CONNECTIONS") {
            config.server.max_connections = parse_positive("FORGE_MAX_CONNECTIONS", &value)?;
        }
        if let Some(value) = lookup("FORGE_PUBLIC_ORIGIN") {
            config.server.public_origin = Some(parse_public_origin(&value)?);
        }
        if let Some(value) = lookup("FORGE_TRUSTED_PROXIES") {
            config.server.trusted_proxies = parse_trusted_proxies(&value)?;
        }
        if let Some(value) = lookup("FORGE_LOG") {
            let filter = value.trim();
            if filter.is_empty() {
                return Err(ConfigError::InvalidValue {
                    key: "FORGE_LOG",
                    value,
                    expected: "a non-empty log filter such as \"info\"",
                });
            }
            config.log.filter = filter.to_owned();
        }
        if let Some(value) = lookup("FORGE_DATABASE_URL") {
            config.database.url = Some(parse_secret_url("FORGE_DATABASE_URL", value)?);
        }
        if let Some(value) = lookup("FORGE_MIGRATION_DATABASE_URL") {
            config.database.migration_url =
                Some(parse_secret_url("FORGE_MIGRATION_DATABASE_URL", value)?);
        }
        // The format default depends on the environment, so derive it only
        // after FORGE_ENV has been applied.
        config.log.format = match lookup("FORGE_LOG_FORMAT") {
            Some(value) => value.parse()?,
            None => LogFormat::default_for(config.environment),
        };

        Ok(config)
    }
}

fn parse_public_origin(value: &str) -> Result<String, ConfigError> {
    let invalid = || ConfigError::InvalidValue {
        key: "FORGE_PUBLIC_ORIGIN",
        value: value.to_owned(),
        expected: "an HTTPS origin (HTTP is allowed only for localhost or a loopback IP), without credentials, path, query, or fragment",
    };
    let url = url::Url::parse(value).map_err(|_| invalid())?;
    let loopback = url.host_str().is_some_and(|host| {
        host.eq_ignore_ascii_case("localhost")
            || host.to_ascii_lowercase().ends_with(".localhost")
            || host
                .parse::<IpAddr>()
                .is_ok_and(|address| address.is_loopback())
    });
    if (url.scheme() != "https" && !(url.scheme() == "http" && loopback))
        || url.username() != ""
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || (url.path() != "/" && !url.path().is_empty())
        || matches!(url.origin(), url::Origin::Opaque(_))
    {
        return Err(invalid());
    }

    Ok(url.origin().ascii_serialization())
}

fn parse_trusted_proxies(value: &str) -> Result<Vec<IpNetwork>, ConfigError> {
    if value.trim().is_empty() {
        return Ok(Vec::new());
    }
    let mut networks = Vec::new();
    for entry in value.split(',') {
        if networks.len() == 64 || entry.trim().is_empty() {
            return Err(ConfigError::InvalidValue {
                key: "FORGE_TRUSTED_PROXIES",
                value: value.to_owned(),
                expected: "up to 64 comma-separated IPv4/IPv6 addresses or CIDRs",
            });
        }
        networks.push(parse_value(
            "FORGE_TRUSTED_PROXIES",
            entry.trim(),
            "an IPv4/IPv6 address or CIDR",
        )?);
    }
    Ok(networks)
}

fn parse_value<T>(key: &'static str, value: &str, expected: &'static str) -> Result<T, ConfigError>
where
    T: FromStr,
{
    value.parse().map_err(|_| ConfigError::InvalidValue {
        key,
        value: value.to_owned(),
        expected,
    })
}

fn parse_positive<T>(key: &'static str, value: &str) -> Result<T, ConfigError>
where
    T: FromStr + Default + PartialOrd,
{
    let parsed = parse_value(key, value, "a positive integer")?;
    if parsed <= T::default() {
        return Err(ConfigError::InvalidValue {
            key,
            value: value.to_owned(),
            expected: "a positive integer",
        });
    }
    Ok(parsed)
}

fn parse_secret_url(key: &'static str, value: String) -> Result<SecretString, ConfigError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(ConfigError::InvalidValue {
            key,
            value,
            expected: "a non-empty PostgreSQL connection URL",
        });
    }
    Ok(SecretString::new(trimmed))
}

/// Configuration loading or validation failure.
#[derive(Debug, Error, Eq, PartialEq)]
pub enum ConfigError {
    /// A known key contains a value outside its contract.
    #[error("invalid value for {key}: {value:?}; expected {expected}")]
    InvalidValue {
        /// Configuration key.
        key: &'static str,
        /// Rejected, non-secret value.
        value: String,
        /// Human-readable contract.
        expected: &'static str,
    },
    /// A `FORGE_*` variable is not a known key, usually a typo.
    #[error("unknown configuration key {key}; expected one of: {known}", known = KNOWN_KEYS.join(", "))]
    UnknownKey {
        /// Rejected variable name.
        key: String,
    },
    /// A known key contains a value that is not valid UTF-8.
    #[error("invalid value for {key}: value is not valid UTF-8")]
    NotUnicode {
        /// Configuration key.
        key: &'static str,
    },
}

mod duration_seconds {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(value: &Duration, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_u64(value.as_secs())
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Duration, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(Duration::from_secs(u64::deserialize(deserializer)?))
    }
}

mod duration_seconds_option {
    use std::time::Duration;

    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(value: &Option<Duration>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match value {
            Some(duration) => serializer.serialize_some(&duration.as_secs()),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Duration>, D::Error>
    where
        D: Deserializer<'de>,
    {
        Option::<u64>::deserialize(deserializer).map(|opt| opt.map(Duration::from_secs))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    #[test]
    fn defaults_are_safe_for_local_development() {
        let config = AppConfig::from_lookup(|_| None).expect("defaults must be valid");

        assert_eq!(config.environment, Environment::Development);
        assert!(config.server.bind.ip().is_loopback());
        assert_eq!(config.server.max_body_bytes, 1_048_576);
        assert!(config.server.trusted_proxies.is_empty());
    }

    #[test]
    fn ip_networks_normalize_match_family_and_cover_prefix_boundaries() {
        let network: IpNetwork = "192.0.2.99/24".parse().expect("valid network");
        assert_eq!(network.to_string(), "192.0.2.0/24");
        assert!(network.contains("192.0.2.255".parse().expect("valid IP")));
        assert!(!network.contains("192.0.3.1".parse().expect("valid IP")));
        assert!(network.contains("::ffff:c000:02ff".parse().expect("valid IP")));

        let all_ipv4: IpNetwork = "198.51.100.1/0".parse().expect("valid /0");
        assert!(all_ipv4.contains("203.0.113.4".parse().expect("valid IP")));
        let one_ipv6: IpNetwork = "2001:db8::1/128".parse().expect("valid /128");
        assert!(one_ipv6.contains("2001:db8::1".parse().expect("valid IP")));
        assert!(!one_ipv6.contains("2001:db8::2".parse().expect("valid IP")));
    }

    #[test]
    fn ip_network_serde_round_trip_revalidates_prefixes() {
        let network: IpNetwork = "10.1.2.3/8".parse().expect("valid network");
        let json = serde_json::to_string(&network).expect("network serializes");
        assert_eq!(json, "\"10.0.0.0/8\"");
        assert_eq!(
            serde_json::from_str::<IpNetwork>(&json).expect("network deserializes"),
            network
        );
        assert!(serde_json::from_str::<IpNetwork>("\"10.0.0.0/33\"").is_err());
    }

    #[test]
    fn provided_values_override_defaults() {
        let values = HashMap::from([
            ("FORGE_ENV", "production"),
            ("FORGE_BIND", "0.0.0.0:8080"),
            ("FORGE_REQUEST_TIMEOUT_SECS", "12"),
            ("FORGE_SHUTDOWN_GRACE_SECS", "7"),
            ("FORGE_MAX_BODY_BYTES", "4096"),
        ]);

        let config = AppConfig::from_lookup(|key| values.get(key).map(ToString::to_string))
            .expect("fixture must be valid");

        assert_eq!(config.environment, Environment::Production);
        assert_eq!(
            config.server.bind,
            "0.0.0.0:8080".parse().expect("valid address")
        );
        assert_eq!(config.server.request_timeout, Duration::from_secs(12));
        assert_eq!(config.server.shutdown_grace, Duration::from_secs(7));
        assert_eq!(config.server.max_body_bytes, 4096);
    }

    #[test]
    fn zero_limits_fail_closed() {
        let error =
            AppConfig::from_lookup(|key| (key == "FORGE_MAX_BODY_BYTES").then(|| "0".to_owned()))
                .expect_err("zero limit must be rejected");

        assert!(matches!(
            error,
            ConfigError::InvalidValue {
                key: "FORGE_MAX_BODY_BYTES",
                ..
            }
        ));
    }

    #[test]
    fn unknown_environment_is_rejected() {
        let error =
            AppConfig::from_lookup(|key| (key == "FORGE_ENV").then(|| "staging-ish".to_owned()))
                .expect_err("unknown environment must be rejected");

        assert!(matches!(
            error,
            ConfigError::InvalidValue {
                key: "FORGE_ENV",
                ..
            }
        ));
    }
    fn vars(pairs: &[(&str, &str)]) -> Vec<(OsString, OsString)> {
        pairs
            .iter()
            .map(|(key, value)| (OsString::from(key), OsString::from(value)))
            .collect()
    }

    #[test]
    fn new_defaults_are_applied() {
        let config = AppConfig::from_vars(Vec::new()).expect("defaults must be valid");

        assert_eq!(config.server.max_connections, 10_000);
        assert_eq!(config.log.filter, "info");
        assert_eq!(config.log.format, LogFormat::Text);
        assert_eq!(config, AppConfig::default());
    }

    #[test]
    fn max_connections_is_parsed_and_must_be_positive() {
        let config = AppConfig::from_vars(vars(&[("FORGE_MAX_CONNECTIONS", "64")]))
            .expect("fixture must be valid");
        assert_eq!(config.server.max_connections, 64);

        for invalid in ["0", "-1", "many"] {
            let error = AppConfig::from_vars(vars(&[("FORGE_MAX_CONNECTIONS", invalid)]))
                .expect_err("invalid connection limit must be rejected");
            assert!(matches!(
                error,
                ConfigError::InvalidValue {
                    key: "FORGE_MAX_CONNECTIONS",
                    ..
                }
            ));
        }
    }

    #[test]
    fn trusted_proxy_configuration_is_strict_and_bounded() {
        let config = AppConfig::from_vars(vars(&[(
            "FORGE_TRUSTED_PROXIES",
            "10.0.0.0/8, 2001:db8::/32, 127.0.0.1",
        )]))
        .expect("valid proxy networks are accepted");
        assert_eq!(config.server.trusted_proxies.len(), 3);
        assert!(
            config.server.trusted_proxies[0]
                .contains("10.5.6.7".parse().expect("valid test address"))
        );

        for invalid in ["10.0.0.0/33", "10.0.0.1,,127.0.0.1", "10.0.0.1/24/8"] {
            let error = AppConfig::from_vars(vars(&[("FORGE_TRUSTED_PROXIES", invalid)]))
                .expect_err("invalid proxy network must fail configuration");
            assert!(matches!(
                error,
                ConfigError::InvalidValue {
                    key: "FORGE_TRUSTED_PROXIES",
                    ..
                }
            ));
        }

        let too_many = (0..65)
            .map(|index| format!("10.0.0.{index}"))
            .collect::<Vec<_>>()
            .join(",");
        assert!(AppConfig::from_vars(vars(&[("FORGE_TRUSTED_PROXIES", &too_many)])).is_err());
    }

    #[test]
    fn public_origin_is_canonical_and_https_except_for_loopback_development() {
        let https = AppConfig::from_vars(vars(&[(
            "FORGE_PUBLIC_ORIGIN",
            "HTTPS://APP.EXAMPLE.TEST/",
        )]))
        .expect("valid public HTTPS origin is accepted");
        assert_eq!(
            https.server.public_origin.as_deref(),
            Some("https://app.example.test")
        );

        let local = AppConfig::from_vars(vars(&[("FORGE_PUBLIC_ORIGIN", "http://127.0.0.1:3000")]))
            .expect("loopback HTTP origin is accepted for development");
        assert_eq!(
            local.server.public_origin.as_deref(),
            Some("http://127.0.0.1:3000")
        );

        for invalid in [
            "http://app.example.test",
            "ftp://app.example.test",
            "https://user@app.example.test",
            "https://app.example.test/login",
            "https://app.example.test/?next=/",
            "https://app.example.test/#section",
            "",
        ] {
            let error = AppConfig::from_vars(vars(&[("FORGE_PUBLIC_ORIGIN", invalid)]))
                .expect_err("unsafe public origin must fail configuration");
            assert!(matches!(
                error,
                ConfigError::InvalidValue {
                    key: "FORGE_PUBLIC_ORIGIN",
                    ..
                }
            ));
        }
    }

    #[test]
    fn log_filter_is_trimmed_and_must_not_be_blank() {
        let config = AppConfig::from_vars(vars(&[("FORGE_LOG", "  warn,my_app=debug ")]))
            .expect("fixture must be valid");
        assert_eq!(config.log.filter, "warn,my_app=debug");

        for blank in ["", "   "] {
            let error = AppConfig::from_vars(vars(&[("FORGE_LOG", blank)]))
                .expect_err("blank filter must be rejected");
            assert!(matches!(
                error,
                ConfigError::InvalidValue {
                    key: "FORGE_LOG",
                    ..
                }
            ));
        }
    }

    #[test]
    fn log_format_defaults_follow_environment() {
        let production = AppConfig::from_vars(vars(&[("FORGE_ENV", "production")]))
            .expect("fixture must be valid");
        assert_eq!(production.log.format, LogFormat::Json);

        let test =
            AppConfig::from_vars(vars(&[("FORGE_ENV", "test")])).expect("fixture must be valid");
        assert_eq!(test.log.format, LogFormat::Text);
    }

    #[test]
    fn explicit_log_format_overrides_environment_default() {
        let config =
            AppConfig::from_vars(vars(&[("FORGE_LOG_FORMAT", "text"), ("FORGE_ENV", "prod")]))
                .expect("fixture must be valid");
        assert_eq!(config.log.format, LogFormat::Text);

        let config = AppConfig::from_vars(vars(&[("FORGE_LOG_FORMAT", " JSON ")]))
            .expect("fixture must be valid");
        assert_eq!(config.log.format, LogFormat::Json);
    }

    #[test]
    fn unknown_log_format_is_rejected() {
        let error = AppConfig::from_vars(vars(&[("FORGE_LOG_FORMAT", "yaml")]))
            .expect_err("unknown format must be rejected");

        assert!(matches!(
            error,
            ConfigError::InvalidValue {
                key: "FORGE_LOG_FORMAT",
                ..
            }
        ));
    }

    #[test]
    fn unknown_forge_keys_fail_closed() {
        let error = AppConfig::from_vars(vars(&[("FORGE_BIDN", "0.0.0.0:8080")]))
            .expect_err("typo must be rejected");

        assert_eq!(
            error,
            ConfigError::UnknownKey {
                key: "FORGE_BIDN".to_owned()
            }
        );
        assert!(error.to_string().contains("FORGE_BIND"));
    }

    #[test]
    fn unrelated_variables_are_ignored() {
        let config = AppConfig::from_vars(vars(&[
            ("PATH", "/usr/bin"),
            ("forge_bidn", "lowercase is not the Forge namespace"),
            ("MY_FORGE_THING", "1"),
            ("FORGE_BIND", "0.0.0.0:9000"),
        ]))
        .expect("unrelated variables must not fail loading");

        assert_eq!(
            config.server.bind,
            "0.0.0.0:9000".parse().expect("valid address")
        );
    }

    #[test]
    fn database_urls_are_available_but_redacted() {
        let config = AppConfig::from_vars(vars(&[
            (
                "FORGE_DATABASE_URL",
                "postgres://runtime:runtime-secret@localhost/app",
            ),
            (
                "FORGE_MIGRATION_DATABASE_URL",
                "postgres://migrator:migration-secret@localhost/app",
            ),
        ]))
        .expect("database URLs should be accepted");

        assert_eq!(
            config
                .database
                .url
                .as_ref()
                .expect("runtime database URL should be present")
                .expose(),
            "postgres://runtime:runtime-secret@localhost/app"
        );
        assert_eq!(
            config
                .database
                .migration_url
                .as_ref()
                .expect("migration database URL should be present")
                .expose(),
            "postgres://migrator:migration-secret@localhost/app"
        );

        let debug = format!("{config:?}");
        assert!(!debug.contains("runtime-secret"));
        assert!(!debug.contains("migration-secret"));
        assert!(debug.contains("[REDACTED]"));

        let json = serde_json::to_string(&config).expect("config should serialize");
        assert!(!json.contains("runtime-secret"));
        assert!(!json.contains("migration-secret"));
        assert!(json.contains("[REDACTED]"));
    }

    #[test]
    fn blank_database_urls_are_rejected() {
        for key in ["FORGE_DATABASE_URL", "FORGE_MIGRATION_DATABASE_URL"] {
            let error = AppConfig::from_vars(vars(&[(key, "   ")]))
                .expect_err("blank database URL must be rejected");

            assert!(matches!(
                error,
                ConfigError::InvalidValue { key: actual, .. } if actual == key
            ));
        }
    }

    #[test]
    fn every_known_key_is_accepted() {
        let values = [
            "production",
            "0.0.0.0:8080",
            "5",
            "5",
            "10",
            "10",
            "https://app.example.test",
            "127.0.0.1,10.0.0.0/8",
            "debug",
            "json",
            "postgres://runtime:secret@localhost/app",
            "postgres://migrator:secret@localhost/app",
        ];
        let pairs: Vec<(&str, &str)> = KNOWN_KEYS.iter().copied().zip(values).collect();
        assert_eq!(pairs.len(), KNOWN_KEYS.len());

        AppConfig::from_vars(vars(&pairs)).expect("every known key must be accepted");
    }

    #[cfg(unix)]
    #[test]
    fn non_unicode_values_are_rejected() {
        use std::os::unix::ffi::OsStringExt;

        let error = AppConfig::from_vars(vec![(
            OsString::from("FORGE_BIND"),
            OsString::from_vec(vec![0x30, 0xff, 0xfe]),
        )])
        .expect_err("non-UTF-8 value must be rejected");

        assert_eq!(error, ConfigError::NotUnicode { key: "FORGE_BIND" });
    }

    #[cfg(unix)]
    #[test]
    fn non_unicode_forge_keys_are_rejected_as_unknown() {
        use std::os::unix::ffi::OsStringExt;

        let error = AppConfig::from_vars(vec![(
            OsString::from_vec(b"FORGE_\xffBIND".to_vec()),
            OsString::from("0.0.0.0:8080"),
        )])
        .expect_err("non-UTF-8 key must be rejected");

        assert!(matches!(error, ConfigError::UnknownKey { .. }));
    }
}
