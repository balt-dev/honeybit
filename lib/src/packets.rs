//! Holds structs for use in sending packets to and from clients.

use fixed::{FixedI16, FixedI8};

#[allow(non_camel_case_types)]
/// Type alias for fixed point fractional i8s.
pub type x8 = FixedI8<5>;
#[allow(non_camel_case_types)]
/// Type alias for fixed point fractional i16s.
pub type x16 = FixedI16<5>;

/// Convenience struct for a position.
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[allow(missing_docs)]
pub struct Position<T> {
    pub x: T, pub y: T, pub z: T
}

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
        position: Position<i16>,
        /// The block's new state. None represents destroying a block.
        state: Option<u8>
    } = 0x05,
    /// Sent to update a player's location.
    /// The player ID always refers to the sender, so it is left out.
    SetPosition {
        /// The player's position.
        position: Position<x16>,
        /// The player's yaw.
        yaw: u8,
        /// The player's pitch.
        pitch: u8
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
        size: Position<i16>
    } = 0x04,
    /// Sent after a block change.
    SetBlock {
        /// The position of the changed block.
        position: Position<i16>,
        /// The changed block's type.
        block: u8
    } = 0x06,
    /// Sent for when a new player is spawning into the world.
    SpawnPlayer {
        /// The player's ID.
        id: i8,
        /// The player's name.
        name: String,
        /// The player's spawn position.
        spawn: Position<x16>,
        /// The player's yaw.
        yaw: u8,
        /// The player's pitch.
        pitch: u8
    } = 0x07,
    /// Sent to teleport a player to a location.
    TeleportPlayer {
        /// The player's ID.
        id: i8,
        /// The player's position.
        position: Position<x16>,
        /// The player's yaw.
        yaw: u8,
        /// The player's pitch.
        pitch: u8
    } = 0x08,
    /// Sent to update a player's position and rotation.
    UpdatePlayerLocation {
        /// The player's ID.
        id: i8,
        /// The player's change in position.
        position_change: Position<x8>,
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
        position_change: Position<x8>
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
