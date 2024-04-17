//! Holds structs pertaining to a world in a server.

use arrayvec::ArrayVec;
use mint::Vector3;
use crate::packets::x16;
use identity_hash::IntMap;

/// A single world within a server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct World {
    /// A hashmap of player IDs to player locations.
    players: IntMap<i8, Location>,
    /// A list of available player IDs.
    available_ids: ArrayVec<i8, 256>,
    /// The stored level data of the world.
    pub level_data: LevelData,
    /// A player's default location.
    default_location: Location
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// Holds the raw level data for a world.
pub struct LevelData {
    /// The raw data backing the world.
    pub raw_data: Vec<u8>,
    /// The level's dimensions.
    pub dimensions: Vector3<u16>
}

impl LevelData {
    /// Creates a new instance of level data.
    #[must_use]
    pub fn new(raw_data: Vec<u8>, dimensions: Vector3<u16>) -> Self {
        Self {
            raw_data,
            dimensions
        }
    }

    /// Gets the ID of a block in the level.
    #[must_use]
    pub fn get(&self, position: Vector3<u16>) -> Option<u8> {
        if position.x >= self.dimensions.x || position.y >= self.dimensions.y || position.z >= self.dimensions.z {
            None
        } else {
            let pos = Vector3 { x: position.x as usize, y: position.y as usize, z: position.z as usize };
            let size = Vector3 { x: self.dimensions.x as usize, y: self.dimensions.y as usize, z: self.dimensions.z as usize};
            self.raw_data.get(pos.y * size.x * size.z + pos.z * size.x + pos.x).copied()
        }
    }

    /// Gets a mutable reference to the ID of a block in the level.
    pub fn get_mut(&mut self, position: Vector3<u16>) -> Option<&mut u8> {
        if position.x >= self.dimensions.x || position.y >= self.dimensions.y || position.z >= self.dimensions.z {
            None
        } else {
            let pos = Vector3 { x: position.x as usize, y: position.y as usize, z: position.z as usize };
            let size = Vector3 { x: self.dimensions.x as usize, y: self.dimensions.y as usize, z: self.dimensions.z as usize};
            self.raw_data.get_mut(pos.y * size.x * size.z + pos.z * size.x + pos.x)
        }
    }
}

impl Default for LevelData {
    fn default() -> Self {
        Self::new(vec![], Vector3 {x: 0, y: 0, z: 0})
    }
}

/// A simple 5x5 world, for use as a default world placeholder.
static DEFAULT_LEVEL_DATA: [u8; 5 * 5 * 5] = [
    // Layer 1
        0x02, 0x02, 0x02, 0x02, 0x02,
        0x02, 0x02, 0x02, 0x02, 0x02,
        0x02, 0x02, 0x02, 0x02, 0x02,
        0x02, 0x02, 0x02, 0x02, 0x02,
        0x02, 0x02, 0x02, 0x02, 0x02,
    // Layer 2
        0x31, 0x15, 0x15, 0x15, 0x00,
        0x28, 0x00, 0x00, 0x00, 0x00,
        0x28, 0x00, 0x00, 0x00, 0x00,
        0x28, 0x00, 0x00, 0x41, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00,
    // Layer 3
        0x25, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00,
    // Layer 4
        0x25, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x41, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00,
    // Layer 5
        0x25, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x41, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00,
];

impl Default for World {
    fn default() -> Self {
        Self {
            players: IntMap::default(),
            available_ids: (i8::MIN ..= i8::MAX).collect(),
            level_data: LevelData::new(Vec::from(DEFAULT_LEVEL_DATA), Vector3 {x: 5, y: 5, z: 5}),
            default_location: Location {
                position: Vector3 {x: x16::from_num(2.5), y: 1.into(), z: x16::from_num(2.5)},
                yaw: 0,
                pitch: 0,
            }
        }
    }
}

impl World {
    /// Initializes a new world, with an empty level.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
    
    /// Gets the number of open slots in the world.
    #[inline]
    #[must_use]
    #[allow(clippy::cast_possible_truncation)]
    pub fn slots(&self) -> u8 {
        self.available_ids.len() as u8
    }
    
    /// Creates a new player in the world. Returns the new ID, or None if the server is full.
    #[inline]
    pub fn create_player(&mut self) -> Option<i8> {
        self.available_ids.pop()
            .inspect(|id| { self.players.insert(*id, self.default_location); })
    }
    
    /// Gets a player's location by their ID.
    #[inline]
    #[must_use]
    pub fn get_player(&self, id: i8) -> Option<Location> {
        self.players.get(&id).copied()
    }

    /// Gets a mutable reference to a player's location by their ID.
    #[inline]
    #[must_use]
    pub fn get_player_mut(&mut self, id: i8) -> Option<&mut Location> {
        self.players.get_mut(&id)
    }
    
    /// Removes a player from the world.
    /// Returns the player's location if they exist.
    #[inline]
    pub fn remove_player(&mut self, id: i8) -> Option<Location> {
        self.players.remove(&id)
    }
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
