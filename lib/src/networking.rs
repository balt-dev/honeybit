//! Module handling the networking side of the server.
#![allow(clippy::cast_sign_loss, clippy::cast_possible_wrap, clippy::cast_possible_truncation, clippy::wildcard_imports, async_fn_in_trait)]

use tokio::io::{AsyncRead, AsyncWrite, AsyncReadExt, AsyncWriteExt};

use std::io::{self, ErrorKind};
use mint::Vector3;
use crate::{
    world::Location,
    packets::*
};

// I'll be real, I could've used serde for this. I just didn't want to.

/// Sealing trait
mod sealed {
    use super::*;
    /// Trait to seal packet type traits
    pub trait Sealed {}
    impl Sealed for u8 {}
    impl Sealed for i8 {}
    impl Sealed for x8 {}
    impl Sealed for u16 {}
    impl Sealed for x16 {}
    impl Sealed for Vec<u8> {}
    impl<T: Sealed> Sealed for Vector3<T> {}
    impl Sealed for String {}
    impl Sealed for Incoming {}
    impl Sealed for Outgoing {}
    impl Sealed for Location {}
}

/// Dictates that this type can be loaded from a packet. This trait is sealed.
pub trait IncomingPacketType : sealed::Sealed {
    #[allow(clippy::missing_errors_doc)]
    /// Dictates how to load this type from a packet.
    async fn load(source: impl AsyncRead + Unpin) -> io::Result<Self> where Self: Sized;
}

/// Dictates that this type can be sent in a packet.
pub trait OutgoingPacketType : sealed::Sealed {
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
        buf.is_ascii()
            // SAFETY: We checked that this is valid ASCII above.
            // By using unchecked, we avoid the unwrap, which would
            // not get optimized out otherwise.
            .then(|| {
                unsafe { String::from_utf8_unchecked(Vec::from(buf)) }
                    .trim_end()
                    .to_string()
            })
            .ok_or(io::Error::from(ErrorKind::InvalidData))
    }
}

impl OutgoingPacketType for String {
    async fn store(&self, mut destination: impl AsyncWrite + Unpin) -> io::Result<()> {
        if !self.is_ascii() {
            return Err(io::Error::from(ErrorKind::InvalidData));
        }
        let slice = self.as_bytes();
        let mut buf = [b' '; 64];
        buf[..slice.len()].copy_from_slice(&slice);
        destination.write_all(&buf).await
    }
}


impl IncomingPacketType for Vec<u8> {
    async fn load(mut source: impl AsyncRead + Unpin) -> io::Result<Self> {
        let length = u16::load(&mut source).await?;
        let mut buf = [0; 1024];
        source.read_exact(&mut buf).await?;
        Ok(Vec::from(&buf[..length as usize]))
    }
}

impl OutgoingPacketType for Vec<u8> {
    async fn store(&self, mut destination: impl AsyncWrite + Unpin) -> io::Result<()> {
        let length = self.len().min(1024) as u16;
        length.store(&mut destination).await?;
        let length = length as usize;
        let mut buf = [0; 1024];
        buf[..length].copy_from_slice(&self.as_slice()[..length]);
        destination.write_all(&buf).await
    }
}

impl IncomingPacketType for Incoming {
    async fn load(mut source: impl AsyncRead + Unpin) -> io::Result<Self> {
        let discriminant = u8::load(&mut source).await?;
        Ok(match discriminant {
            0x00 => {
                let ret = Incoming::PlayerIdentification {
                    version: u8::load(&mut source).await?,
                    username: String::load(&mut source).await?,
                    key: String::load(&mut source).await?
                };
                let _ = u8::load(source).await?;
                ret
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
            0x0d => {
                let _ = u8::load(&mut source).await?;
                Incoming::Message {
                    message: String::load(source).await?
                }
            }
            _ => return Err(
                io::Error::from(ErrorKind::InvalidData)
            )
        })
    }
}

impl OutgoingPacketType for Outgoing {
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
            Outgoing::LevelDataChunk { data_chunk, percent_complete } => {
                0x3u8.store(&mut destination).await?;
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
            }
        }
    }
}
