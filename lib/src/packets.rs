//! Holds structs for use in sending packets to and from clients.

use fixed::{
    FixedI8, FixedU16,
    types::extra::U5
};
pub use mint::Vector3;
use crate::world::Location;

#[allow(non_camel_case_types)]
/// Type alias for fixed point fractional i8s.
pub type x8 = FixedI8<U5>;
#[allow(non_camel_case_types)]
/// Type alias for fixed point fractional u16s.
pub type x16 = FixedU16<U5>;

/// Packets going from the client to the server.
#[repr(u8)]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Incoming {
    /// Sent by a player joining the server.
    PlayerIdentification {
        /// The protocol version. Should always be 0x07.
        version: u8,
        /// The player's username.
        username: String,
        /// The player's verification key.
        key: String
    } = 0x00,
    /// Sent when a user changes a block.
    SetBlock {
        /// The position of the changed block.
        position: Vector3<u16>,
        /// The block's new state. 0x00 represents destroying a block.
        state: u8
    } = 0x05,
    /// Sent to update a player's location.
    /// The player ID always refers to the sender, so it is left out.
    SetLocation {
        /// The player's new position and rotation.
        location: Location
    } = 0x08,
    /// Sent when a chat message is sent.
    Message {
        /// The chat message sent.
        message: String
    } = 0x0D
}

/// Packets going from the server to the client.
#[repr(u8)]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Outgoing {
    /// Response to a player joining the game.
    ServerIdentification {
        /// The protocol version. Should be 0x07.
        version: u8,
        /// The server's name.
        name: String,
        /// The server's MOTD.
        motd: String,
        /// Whether the player is an operator or not.
        operator: bool
    } = 0x00,
    /// Periodically sent to clients.
    Ping = 0x01,
    /// Notifies a player of incoming level data.
    LevelInit = 0x02,
    /// Contains a chunk of level data.
    LevelDataChunk {
        /// A chunk of the gzipped level data. May not be larger than 1024 bytes.
        data_chunk: Vec<u8>,
        /// How close the level data is to being fully sent.
        percent_complete: u8
    } = 0x03,
    /// Sent after level data is done sending, containing map dimensions.
    LevelFinalize {
        /// The size of the map.
        size: Vector3<u16>
    } = 0x04,
    /// Sent after a block change.
    SetBlock {
        /// The position of the changed block.
        position: Vector3<u16>,
        /// The changed block's type.
        block: u8
    } = 0x06,
    /// Sent for when a new player is spawning into the world.
    SpawnPlayer {
        /// The player's ID.
        id: i8,
        /// The player's name.
        name: String,
        /// The player's spawn position and rotation.
        location: Location
    } = 0x07,
    /// Sent to teleport a player to a location.
    TeleportPlayer {
        /// The player's ID.
        id: i8,
        /// The player's new position and rotation.
        location: Location
    } = 0x08,
    /// Sent to update a player's position and rotation.
    UpdatePlayerLocation {
        /// The player's ID.
        id: i8,
        /// The player's change in position.
        position_change: Vector3<x8>,
        /// The player's new yaw.
        yaw: u8,
        /// The player's new pitch.
        pitch: u8,
    } = 0x09,
    /// Sent to update a player's position.
    UpdatePlayerPosition {
        /// The player's ID.
        id: i8,
        /// The player's change in position.
        position_change: Vector3<x8>
    } = 0x0a,
    /// Sent to update a player's rotation.
    UpdatePlayerRotation {
        /// The player's ID.
        id: i8,
        /// The player's new yaw.
        yaw: u8,
        /// The player's new pitch.
        pitch: u8
    } = 0x0b,
    /// Sent when another player disconnects.
    DespawnPlayer {
        /// The despawning player's ID.
        id: i8,
    } = 0x0c,
    /// Sent to players when a message is sent in chat.
    Message {
        /// The player who sent the message.
        id: i8,
        /// The message sent.
        message: String
    } = 0x0d,
    /// Sent to a player to disconnect them.
    Disconnect {
        /// The reason the player is disconnecting.
        reason: String
    } = 0x0e,
    /// Sent when a player's operator status changes.
    UpdateUser {
        /// Whether the player is an operator or not.
        operator: bool
    } = 0x0F
}
