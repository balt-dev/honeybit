//! Handles the actual server.

use std::{
    net::IpAddr,
    time::Duration,
    collections::HashMap
};

use rand::{Rng, rngs::StdRng};

/// A trait to help generate valid salts for the server.
pub trait SaltExt {
    /// Generate a salt.
    fn salt(&mut self) -> String;
}

impl SaltExt for StdRng {
    #[inline]
    fn salt(&mut self) -> String {
        const SALT_MIN: u128 =    768_909_704_948_766_668_552_634_368; // base62::decode("1000000000000000").unwrap();
        const SALT_MAX: u128 = 47_672_401_706_823_533_450_263_330_815; // base62::decode("zzzzzzzzzzzzzzzz").unwrap();
        let num: u128 = self.gen_range(SALT_MIN ..= SALT_MAX);
        base62::encode(num)
    }
}


/// Configuration for a server.
#[derive(Debug, Clone, PartialEq, Eq)]
#[derive(serde::Serialize, serde::Deserialize)]
pub struct Config {
    /// How long the server will wait for a client to respond to a packet.
    pub packet_timeout: Duration,
    /// How often the server will send pings to clients.
    pub ping_spacing: Duration,
    /// The default world to connect to.
    pub default_world: String,
    /// A mapping of banned IPs to their ban reasons.
    pub banned_ips: HashMap<IpAddr, String>,
    /// A mapping of banned usernames to their ban reasons.
    pub banned_users: HashMap<String, String>,
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
    pub public: bool
}
