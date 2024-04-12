//! Handles the actual server.

use std::{
    collections::{
        HashMap,
        HashSet
    },
    time::Duration,
    net::IpAddr
};
use crate::world::World;

/// An instance of a server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Server {
    /// A mapping of names to worlds in the server.
    pub worlds: HashMap<String, World>,
    /// The configuration for the server.
    pub config: Config,
    /// The salt used to verify users.
    pub salt: String
}


/// Configuration for a server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// How long the server will wait for a client to respond to a packet.
    pub packet_timeout: Duration,
    /// How often the server will send pings to clients.
    pub ping_spacing: Duration,
    /// The default world to connect to.
    pub default_world: String,
    /// A list of banned IPs.
    pub banned_ips: HashSet<IpAddr>,
    /// Whether to verify users or not.
    pub verify_users: bool,
    /// The server name to display in the server list.
    pub name: String,
    /// A URL linking to the heartbeat page the server will ping.
    pub url: String,
    /// The port to host the server on.
    pub port: u16
}
