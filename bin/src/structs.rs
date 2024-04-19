use std::collections::{HashMap, HashSet};
use std::net::IpAddr;
use std::time::Duration;

/// Configuration for a server.
#[derive(Debug, Clone, PartialEq, Eq)]
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Config {
    /// How long the server will wait for a client to respond to a ping.
    pub packet_timeout: Duration,
    /// How often the server will send pings to clients.
    pub ping_spacing: Duration,
    /// The default world to connect to.
    pub default_world: String,
    /// A mapping of banned IPs to their ban reasons.
    pub banned_ips: HashMap<IpAddr, String>,
    /// A mapping of banned usernames to their ban reasons.
    pub banned_users: HashMap<String, String>,
    /// A set of usernames that are operators.
    pub operators: HashSet<String>,
    /// The amount of salts to keep for verifying users.
    ///
    /// If this is zero, then users will not be verified.
    pub kept_salts: usize,
    /// The server name to display in the server list.
    pub name: String,
    /// A URL linking to the heartbeat server the server will ping.
    ///
    /// If this is empty, then the heartbeat URL will not be pinged.
    ///
    /// Note that leaving this empty AND setting `kept_salts` to above 0
    /// will create a situation where players will not be able to be
    /// verified! This will cause a runtime error.
    pub heartbeat_url: String,
    /// The amount of times to retry connecting to the heartbeat server.
    pub heartbeat_retries: usize,
    /// How often the server will send pings to the heartbeat server.
    pub heartbeat_spacing: Duration,
    /// How long the server will wait for sending pings to the heartbeat server before trying again.
    pub heartbeat_timeout: Duration,
    /// The port to host the server on.
    pub port: u16,
    /// The maximum amount of players allowed on the server.
    ///
    /// If this is set to 0, then the amount will be unlimited.
    pub max_players: usize,
    /// Whether the server should be public in the server list.
    pub public: bool,
    /// The server's MOTD.
    pub motd: String
}

impl Default for Config {
    fn default() -> Self {
        Config {
            packet_timeout: Duration::from_secs(10),
            ping_spacing: Duration::from_millis(500),
            default_world: "world".into(),
            banned_ips: HashMap::default(),
            banned_users: HashMap::default(),
            kept_salts: 0,
            name: "<Unnamed Server>".to_string(),
            heartbeat_url: String::new(),
            heartbeat_retries: 5,
            heartbeat_spacing: Duration::from_secs(5),
            heartbeat_timeout: Duration::from_secs(5),
            port: 25565,
            max_players: 64,
            public: false,
            operators: HashSet::new(),
            motd: "Running on Oxine".into(),
        }
    }
}
