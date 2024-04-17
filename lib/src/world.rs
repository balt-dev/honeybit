use mint::Vector3;
use crate::packets::x16;


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
