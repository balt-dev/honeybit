//! Holds structs for use in sending packets to and from clients.
#![allow(clippy::cast_possible_wrap, clippy::cast_sign_loss)]


use std::collections::HashMap;
use std::sync::atomic::{AtomicU16, AtomicU8, Ordering};
use fixed::{
    FixedI8, FixedU16,
    types::extra::U5
};
pub use mint::Vector3;

#[allow(non_camel_case_types)]
/// Type alias for fixed point fractional i8s.
pub type x8 = FixedI8<U5>;
#[allow(non_camel_case_types)]
/// Type alias for fixed point fractional u16s.
pub type x16 = FixedU16<U5>;

bitflags! {
    /// A bitfield of supported extensions.
    #[derive(Debug, Clone, PartialEq, Eq, Copy, Hash)]
    pub struct SupportedExtensions: u32 {
        const FULL_CP437 = 0x1;
        const LONGER_MESSAGES = 0x2;
        const EMOTE_FIX = 0x4;
    }
}

/// Packets going from the client to the server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Incoming {
    /// Sent by a player joining the server.
    PlayerIdentification {
        /// The protocol version. Should always be 0x07.
        version: u8,
        /// The player's username.
        username: String,
        /// The player's verification key.
        key: String,
        /// Whether the player supports CPE.
        cpe_supported: bool
    },
    /// Sent when a user changes a block.
    SetBlock {
        /// The position of the changed block.
        position: Vector3<u16>,
        /// The block's new state. 0x00 represents destroying a block.
        state: u8
    },
    /// Sent to update a player's location.
    /// The player ID always refers to the sender, so it is left out.
    SetLocation {
        /// The player's new position and rotation.
        location: Location
    },
    /// Sent when a chat message is sent.
    Message {
        /// The chat message sent.
        message: [u8; 64],
        /// Whether this chat message should be appended to the last.
        append: bool
    },
    /// Sent to notify the server that the player supports CPE, and which extensions it supports.
    ExtInfoEntry {
        /// A bitfield of supported versions.
        supported_exts: SupportedExtensions
    }
}

/// A list of supported CPE extensions.
static SUPPORTED_EXTS: Lazy<HashMap<String, (SupportedExtensions, u32)>> = Lazy::new(|| HashMap::from([
    ("FullCP437".into(), (SupportedExtensions::FULL_CP437, 1)),
    ("LongerMessages".into(), (SupportedExtensions::LONGER_MESSAGES, 1)),
    ("EmoteFix".into(), (SupportedExtensions::EMOTE_FIX, 1))
]));

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
    },
    /// Periodically sent to clients.
    Ping,
    /// Notifies a player of incoming level data.
    LevelInit,
    /// Contains a chunk of level data.
    LevelDataChunk {
        /// How many bytes are initialized in the chunk.
        data_length: u16,
        /// A chunk of the gzipped level data.
        data_chunk: Box<[u8; 1024]>,
        /// How close the level data is to being fully sent.
        percent_complete: u8
    },
    /// Sent after level data is done sending, containing map dimensions.
    LevelFinalize {
        /// The size of the map.
        size: Vector3<u16>
    },
    /// Sent after a block change.
    SetBlock {
        /// The position of the changed block.
        position: Vector3<u16>,
        /// The changed block's type.
        block: u8
    },
    /// Sent for when a new player is spawning into the world.
    SpawnPlayer {
        /// The player's ID.
        id: i8,
        /// The player's name.
        name: String,
        /// The player's spawn position and rotation.
        location: Location
    },
    /// Sent to teleport a player to a location.
    TeleportPlayer {
        /// The player's ID.
        id: i8,
        /// The player's new position and rotation.
        location: Location
    },
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
    },
    /// Sent to update a player's position.
    UpdatePlayerPosition {
        /// The player's ID.
        id: i8,
        /// The player's change in position.
        position_change: Vector3<x8>
    },
    /// Sent to update a player's rotation.
    UpdatePlayerRotation {
        /// The player's ID.
        id: i8,
        /// The player's new yaw.
        yaw: u8,
        /// The player's new pitch.
        pitch: u8
    },
    /// Sent when another player disconnects.
    DespawnPlayer {
        /// The despawning player's ID.
        id: i8,
    },
    /// Sent to players when a message is sent in chat.
    Message {
        /// The player who sent the message.
        id: i8,
        /// The message sent.
        message: [u8; 64]
    },
    /// Sent to a player to disconnect them.
    Disconnect {
        /// The reason the player is disconnecting.
        reason: String
    },
    /// Sent when a player's operator status changes.
    UpdateUser {
        /// Whether the player is an operator or not.
        operator: bool
    },
    /// Sent to notify players that the server supports CPE.
    ExtInfoEntry
}


/// A single player's position and rotation.
#[derive(Copy, Clone, Ord, PartialOrd, Eq, PartialEq, Debug, Hash)]
pub struct Location {
    /// The player's position.
    pub position: Vector3<x16>,
    /// The player's yaw.
    pub yaw: u8,
    /// The player's pitch.
    pub pitch: u8
}

impl Default for Location {
    fn default() -> Self {
        Self {
            position: Vector3 { x: 0.into(), y: 0.into(), z: 0.into() },
            yaw: 0,
            pitch: 0
        }
    }
}

/// A single player's position and rotation, stored to allow atomic operations.
#[derive(Debug)]
pub struct AtomicLocation {
    /// The player's position.
    pub position: Vector3<AtomicU16>,
    /// The player's yaw.
    pub yaw: AtomicU8,
    /// The player's pitch.
    pub pitch: AtomicU8
}

impl AtomicLocation {
    /// Updates the atomic location from the location's fields.
    pub fn update(&self, location: Location) {
        self.position.x.store(location.position.x.to_bits(), Ordering::Relaxed);
        self.position.y.store(location.position.y.to_bits(), Ordering::Relaxed);
        self.position.z.store(location.position.z.to_bits(), Ordering::Relaxed);
        self.yaw.store(location.yaw, Ordering::Relaxed);
        self.pitch.store(location.pitch, Ordering::Relaxed);
    }
}

impl From<&AtomicLocation> for Location {
    fn from(value: &AtomicLocation) -> Self {
        Location {
            position: Vector3 {
                x: x16::from_bits(value.position.x.load(Ordering::Relaxed)),
                y: x16::from_bits(value.position.y.load(Ordering::Relaxed)),
                z: x16::from_bits(value.position.z.load(Ordering::Relaxed))
            },
            yaw: value.yaw.load(Ordering::Relaxed),
            pitch: value.pitch.load(Ordering::Relaxed)
        }
    }
}

impl From<Location> for AtomicLocation {
    fn from(value: Location) -> Self {
        AtomicLocation {
            position: Vector3 {
                x: value.position.x.to_bits().into(),
                y: value.position.y.to_bits().into(),
                z: value.position.z.to_bits().into(),
            },
            yaw: value.yaw.into(),
            pitch: value.pitch.into()
        }
    }
}
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use std::io::{self, ErrorKind};
use bitflags::bitflags;
use codepage_437::{BorrowFromCp437, ToCp437};
use once_cell::sync::Lazy;

/// Dictates that this type can be loaded from a packet. This trait is sealed.
pub trait IncomingPacketType {
    #[allow(clippy::missing_errors_doc)]
    /// Dictates how to load this type from a packet.
    async fn load(source: impl AsyncRead + Unpin) -> io::Result<Self> where Self: Sized;
}

/// Dictates that this type can be sent in a packet.
pub trait OutgoingPacketType {
    #[allow(clippy::missing_errors_doc)]
    /// Dictates how to store this type in a packet.
    async fn store(&self, destination: impl AsyncWrite + Unpin) -> io::Result<()>;
}

impl IncomingPacketType for u8 {
    async fn load(mut source: impl AsyncRead + Unpin) -> io::Result<Self> {
        let mut buf = [0];
        source.read_exact(&mut buf).await?;
        Ok(buf[0])
    }
}

impl OutgoingPacketType for u8 {
    async fn store(&self, mut destination: impl AsyncWrite + Unpin) -> io::Result<()> {
        destination.write_all(&[*self]).await
    }
}

impl IncomingPacketType for i8 {
    async fn load(mut source: impl AsyncRead + Unpin) -> io::Result<Self> {
        let mut buf = [0];
        source.read_exact(&mut buf).await?;
        Ok(buf[0] as i8)
    }
}

impl OutgoingPacketType for i8 {
    async fn store(&self, mut destination: impl AsyncWrite + Unpin) -> io::Result<()> {
        destination.write_all(&[*self as u8]).await
    }
}

impl IncomingPacketType for u16 {
    async fn load(mut source: impl AsyncRead + Unpin) -> io::Result<Self> {
        let mut buf = [0, 0];
        source.read_exact(&mut buf).await?;
        Ok(u16::from_be_bytes(buf))
    }
}

impl OutgoingPacketType for u16 {
    async fn store(&self, mut destination: impl AsyncWrite + Unpin) -> io::Result<()> {
        destination.write_all(&self.to_be_bytes()).await
    }
}

impl IncomingPacketType for u32 {
    async fn load(mut source: impl AsyncRead + Unpin) -> io::Result<Self> {
        let mut buf = [0; 4];
        source.read_exact(&mut buf).await?;
        Ok(u32::from_be_bytes(buf))
    }
}

impl OutgoingPacketType for u32 {
    async fn store(&self, mut destination: impl AsyncWrite + Unpin) -> io::Result<()> {
        destination.write_all(&self.to_be_bytes()).await
    }
}

impl IncomingPacketType for x8 {
    async fn load(mut source: impl AsyncRead + Unpin) -> io::Result<Self> {
        let mut buf = [0];
        source.read_exact(&mut buf).await?;
        Ok(x8::from_bits(buf[0] as i8))
    }
}

impl OutgoingPacketType for x8 {
    async fn store(&self, mut destination: impl AsyncWrite + Unpin) -> io::Result<()> {
        destination.write_all(&[self.to_bits() as u8]).await
    }
}

impl IncomingPacketType for x16 {
    async fn load(mut source: impl AsyncRead + Unpin) -> io::Result<Self> {
        let mut buf = [0, 0];
        source.read_exact(&mut buf).await?;
        Ok(x16::from_be_bytes(buf))
    }
}

impl OutgoingPacketType for x16 {
    async fn store(&self, mut destination: impl AsyncWrite + Unpin) -> io::Result<()> {
        destination.write_all(&self.to_be_bytes()).await
    }
}

impl<T: IncomingPacketType> IncomingPacketType for Vector3<T> {
    async fn load(mut source: impl AsyncRead + Unpin) -> io::Result<Self> {
        Ok(Vector3{
            x: T::load(&mut source).await?,
            y: T::load(&mut source).await?,
            z: T::load(source).await?,
        })
    }
}

impl<T: OutgoingPacketType> OutgoingPacketType for Vector3<T> {
    async fn store(&self, mut destination: impl AsyncWrite + Unpin) -> io::Result<()> {
        self.x.store(&mut destination).await?;
        self.y.store(&mut destination).await?;
        self.z.store(destination).await
    }
}

impl IncomingPacketType for Location {
    async fn load(mut source: impl AsyncRead + Unpin) -> io::Result<Self> {
        Ok(Location {
            position: Vector3::<x16>::load(&mut source).await?,
            yaw: u8::load(&mut source).await?,
            pitch: u8::load(source).await?,
        } )
    }
}

impl OutgoingPacketType for Location {
    async fn store(&self, mut destination: impl AsyncWrite + Unpin) -> io::Result<()> {
        self.position.store(&mut destination).await?;
        self.yaw.store(&mut destination).await?;
        self.pitch.store(destination).await
    }
}

impl IncomingPacketType for String {
    async fn load(mut source: impl AsyncRead + Unpin) -> io::Result<Self> {
        let mut buf = [0; 64];
        source.read_exact(&mut buf).await?;
        let borrow = String::borrow_from_cp437(&buf, &codepage_437::CP437_WINGDINGS);
        // Conversion from a buffer ot CP437 is infallible
        Ok(borrow.trim_end().into())
    }
}

impl OutgoingPacketType for String {
    async fn store(&self, mut destination: impl AsyncWrite + Unpin) -> io::Result<()> {
        let Ok(slice) = self.to_cp437(&codepage_437::CP437_WINGDINGS) else {
            return Err(io::Error::from(ErrorKind::InvalidData));
        };
        let mut buf = [b' '; 64];
        let trunc_len = slice.len().min(64);
        buf[..trunc_len].copy_from_slice(&slice[..trunc_len]);
        destination.write_all(&buf).await
    }
}


impl<const SIZE: usize> IncomingPacketType for [u8; SIZE] {
    async fn load(mut source: impl AsyncRead + Unpin) -> io::Result<Self> {
        let mut buf = [0; SIZE];
        source.read_exact(&mut buf).await?;
        Ok(buf)
    }
}

impl<const SIZE: usize> OutgoingPacketType for [u8; SIZE] {
    async fn store(&self, mut destination: impl AsyncWrite + Unpin) -> io::Result<()> {
        destination.write_all(self).await
    }
}

impl IncomingPacketType for Incoming {
    async fn load(mut source: impl AsyncRead + Unpin) -> io::Result<Self> {
        let discriminant = u8::load(&mut source).await?;
        Ok(match discriminant {
            0x00 => Incoming::PlayerIdentification {
                version: u8::load(&mut source).await?,
                username: String::load(&mut source).await?,
                key: String::load(&mut source).await?,
                cpe_supported: u8::load(source).await? == 0x42
            },
            0x05 => {
                let position = Vector3::<u16>::load(&mut source).await?;
                let mode = u8::load(&mut source).await? != 0;
                let id = u8::load(source).await?;
                Incoming::SetBlock {
                    position,
                    state: if mode {id} else {0}
                }
            },
            0x08 => {
                let _ = u8::load(&mut source).await?;
                Incoming::SetLocation {
                    location: Location::load(&mut source).await?
                }
            },
            0x0d => Incoming::Message {
                append: u8::load(&mut source).await? == 1,
                message: <[u8; 64]>::load(source).await?
            },
            0x10 => {
                let _ = String::load(&mut source).await?; // We don't care what the client is
                let count = u16::load(&mut source).await?;
                debug!("Client supports {count}");
                let mut supported_exts = SupportedExtensions::empty();
                for _ in 0..count {
                    let id = u8::load(&mut source).await?;
                    if id != 0x11 {
                        return Err(
                            io::Error::new(ErrorKind::InvalidData, format!("Expected ExtEntry (0x11), got 0x{id:02x}"))
                        )
                    }
                    let name = String::load(&mut source).await?;
                    let their_version = u32::load(&mut source).await?;
                    debug!("Client supports extension \"{name}\" with version {their_version}");
                    let Some((mask, our_version)) = SUPPORTED_EXTS.get(&name)
                        else { continue };
                    if *our_version == their_version {
                        supported_exts |= *mask;
                    }
                }
                Incoming::ExtInfoEntry { supported_exts }
            },
            dis => return Err(
                io::Error::new(ErrorKind::InvalidData, format!("Invalid packet discriminator 0x{dis:02x}"))
            )
        })
    }
}

impl OutgoingPacketType for Outgoing {
    #[allow(clippy::cast_possible_truncation)]
    async fn store(&self, mut destination: impl AsyncWrite + Unpin) -> io::Result<()> {
        match self {
            Outgoing::ServerIdentification { version, name, motd, operator } => {
                0x0u8.store(&mut destination).await?;
                version.store(&mut destination).await?;
                name.store(&mut destination).await?;
                motd.store(&mut destination).await?;
                (if *operator { 0x64u8 } else { 0x00u8 }).store(destination).await
            },
            Outgoing::Ping => 0x1u8.store(destination).await,
            Outgoing::LevelInit => 0x2u8.store(destination).await,
            Outgoing::LevelDataChunk { data_length, data_chunk, percent_complete } => {
                0x3u8.store(&mut destination).await?;
                data_length.store(&mut destination).await?;
                data_chunk.store(&mut destination).await?;
                percent_complete.store(destination).await
            },
            Outgoing::LevelFinalize { size } => {
                0x4u8.store(&mut destination).await?;
                size.store(destination).await
            },
            Outgoing::SetBlock { position, block } => {
                0x6u8.store(&mut destination).await?;
                position.store(&mut destination).await?;
                block.store(destination).await
            },
            Outgoing::SpawnPlayer { id, name, location } => {
                0x7u8.store(&mut destination).await?;
                id.store(&mut destination).await?;
                name.store(&mut destination).await?;
                location.store(&mut destination).await
            },
            Outgoing::TeleportPlayer { id, location } => {
                0x8u8.store(&mut destination).await?;
                id.store(&mut destination).await?;
                location.store(&mut destination).await
            },
            Outgoing::UpdatePlayerLocation { id, position_change, yaw, pitch } => {
                0x9u8.store(&mut destination).await?;
                id.store(&mut destination).await?;
                position_change.store(&mut destination).await?;
                yaw.store(&mut destination).await?;
                pitch.store(destination).await
            },
            Outgoing::UpdatePlayerPosition { id, position_change } => {
                0xau8.store(&mut destination).await?;
                id.store(&mut destination).await?;
                position_change.store(destination).await
            },
            Outgoing::UpdatePlayerRotation { id, yaw, pitch } => {
                0xbu8.store(&mut destination).await?;
                id.store(&mut destination).await?;
                yaw.store(&mut destination).await?;
                pitch.store(destination).await
            },
            Outgoing::DespawnPlayer { id } => {
                0xcu8.store(&mut destination).await?;
                id.store(destination).await
            },
            Outgoing::Message { id, message } => {
                0xdu8.store(&mut destination).await?;
                id.store(&mut destination).await?;
                message.store(destination).await
            },
            Outgoing::Disconnect { reason } => {
                0xeu8.store(&mut destination).await?;
                reason.store(destination).await
            },
            Outgoing::UpdateUser { operator } => {
                0xfu8.store(&mut destination).await?;
                (if *operator {0x64} else {0u8}).store(destination).await
            },
            Outgoing::ExtInfoEntry => {
                0x10u8.store(&mut destination).await?;
                "Honeybit".to_string().store(&mut destination).await?;
                (SUPPORTED_EXTS.len() as u16).store(&mut destination).await?;
                for (name, (_, version)) in SUPPORTED_EXTS.iter() {
                    0x11u8.store(&mut destination).await?;
                    name.to_string().store(&mut destination).await?;
                    (*version).store(&mut destination).await?;
                    debug!("Server supports extension \"{name}\" with version {version}");
                }
                Ok(())
            }
        }
    }
}
