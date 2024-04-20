//! Holds structs pertaining to a world in a server.

use std::{io, path::PathBuf, sync::{Arc, atomic::Ordering}};
use std::io::{Cursor, Read, Write};
use arrayvec::ArrayVec;
use flate2::Compression;
use flate2::read::GzEncoder;
use mint::Vector3;
use crate::{
    packets::Location,
    player::{WeakPlayer, Command},
};
use identity_hash::IntMap;
use itertools::Itertools;
use tokio::sync::Mutex as TokioMutex;
use parking_lot::Mutex;
use tokio::sync::mpsc::Sender;
use crate::packets::Outgoing;


/// A single world within a server.
#[derive(Debug, Clone)]
pub struct World {
    /// The world's filepath.
    pub filepath: Arc<PathBuf>,
    /// A hashmap of player IDs to players.
    pub players: Arc<Mutex<IntMap<i8, WeakPlayer>>>,
    /// A list of available player IDs.
    pub available_ids: Arc<Mutex<ArrayVec<i8, 256>>>,
    /// The stored data of the world.
    pub world_data: Arc<TokioMutex<WorldData>>,
}

/// A holding class for a serialized level .DAT file.
#[derive(Debug, Clone)]
pub struct WorldData {
    /// The raw level data.
    pub level_data: LevelData,
    /// The player spawn point.
    pub spawn_point: Location,
    /// The world's name.
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
/// Holds the raw level data for a world.
pub struct LevelData {
    /// The raw data backing the world.
    pub raw_data: Vec<u8>,
    /// The level's dimensions.
    pub dimensions: Vector3<u16>,
}

impl LevelData {
    /// Creates a new instance of level data.
    #[must_use]
    pub fn new(raw_data: Vec<u8>, dimensions: Vector3<u16>) -> Self {
        Self {
            raw_data,
            dimensions,
        }
    }

    /// Gets the ID of a block in the level.
    #[must_use]
    pub fn get(&self, position: Vector3<u16>) -> Option<u8> {
        if position.x >= self.dimensions.x || position.y >= self.dimensions.y || position.z >= self.dimensions.z {
            None
        } else {
            let pos = Vector3 { x: position.x as usize, y: position.y as usize, z: position.z as usize };
            let size = Vector3 { x: self.dimensions.x as usize, y: self.dimensions.y as usize, z: self.dimensions.z as usize };
            self.raw_data.get(pos.y * size.x * size.z + pos.z * size.x + pos.x).copied()
        }
    }

    /// Gets a mutable reference to the ID of a block in the level.
    pub fn get_mut(&mut self, position: Vector3<u16>) -> Option<&mut u8> {
        if position.x >= self.dimensions.x || position.y >= self.dimensions.y || position.z >= self.dimensions.z {
            debug!("{position:?}, {:?}", self.dimensions);
            None
        } else {
            let pos = Vector3 { x: position.x as usize, y: position.y as usize, z: position.z as usize };
            let size = Vector3 { x: self.dimensions.x as usize, y: self.dimensions.y as usize, z: self.dimensions.z as usize };
            self.raw_data.get_mut(pos.y * size.x * size.z + pos.z * size.x + pos.x)
        }
    }
}

impl Default for LevelData {
    fn default() -> Self {
        Self::new(vec![], Vector3 { x: 0, y: 0, z: 0 })
    }
}

impl Default for World {
    fn default() -> Self {
        Self {
            filepath: Arc::default(),
            players: Arc::default(),
            available_ids: Arc::new(Mutex::new(
                [0].into_iter().collect()
            )),
            world_data: Arc::new(TokioMutex::new(
                WorldData {
                    level_data: LevelData::default(),
                    spawn_point: Location::default(),
                    name: String::new(),
                }
            )),
        }
    }
}


struct WorldEncoder<'inner> {
    inner: Cursor<&'inner [u8]>,
    length_read: bool,
}

impl<'inner> WorldEncoder<'inner> {
    fn new(slice: &'inner [u8]) -> Self {
        Self {
            inner: Cursor::new(slice),
            length_read: false,
        }
    }
}

impl<'inner> Read for WorldEncoder<'inner> {
    #[allow(clippy::cast_possible_truncation)]
    fn read(&mut self, mut buf: &mut [u8]) -> io::Result<usize> {
        if self.length_read {
            self.inner.read(buf)
        } else {
            let len = self.inner.get_ref().len() as u32;
            let slice = len.to_be_bytes();
            buf.write_all(&slice)?;
            self.length_read = true;
            Ok(4)
        }
    }
}


impl World {
    /// Initializes a new world, with an empty level.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Checks if the world is full.
    #[inline]
    #[must_use]
    #[allow(clippy::cast_possible_truncation)]
    pub fn is_full(&self) -> bool {
        self.available_ids.lock().is_empty()
    }

    /// Creates a new player in the world. Returns the new ID, or None if the server is full.
    ///
    ///
    #[inline]
    pub async fn add_player(&self, player: WeakPlayer, packet_send: Sender<Outgoing>) -> Option<i8> {
        self.collect_garbage();

        // We hold the lock for the entire time here so that
        // any block updates aren't pushed until the world data is done being sent
        let data_lock = self.world_data.lock().await;

        let dimensions = data_lock.level_data.dimensions;


        // GZip level data
        let data_slice = data_lock.level_data.raw_data.as_slice();

        debug!("{} bytes to compress", data_slice.len());

        let Ok(()) = packet_send.send(Outgoing::LevelInit).await else { return None; };

        let mut encoder = GzEncoder::new(WorldEncoder::new(data_slice), Compression::fast());

        // For some reason, streaming the encoded data refused to work.
        // Really annoying but oh well I guess >:/
        let mut encoded_data = Vec::new();
        encoder.read_to_end(&mut encoded_data).expect("reading into Vec should never fail");

        let mut buf = [0; 1024];

        let iter = encoded_data.chunks(1024);
        let chunk_count = iter.len();

        for (i, chunk) in iter.enumerate() {
            buf[..chunk.len()].copy_from_slice(chunk);

            #[allow(clippy::pedantic)]
                let Ok(()) = packet_send.send(Outgoing::LevelDataChunk {
                data_length: chunk.len() as u16,
                data_chunk: Box::new(buf),
                percent_complete: ((i as f32) / (chunk_count as f32) * 100.0) as u8,
            }).await else { return None; };
        }

        let Ok(()) = packet_send.send(Outgoing::LevelFinalize { size: dimensions }).await
            else { return None; };

        drop(data_lock);

        let id = self.available_ids.lock().pop()?;

        let player_name = player.username.upgrade()?.lock().clone();

        player.id.upgrade()?.store(id, Ordering::Relaxed);
        {
            let world = player.world.upgrade()?;
            if world.is_locked() { return None; }
            let mut lock = world.lock();
            *lock = self.clone();
        }

        let default_location = {
            let lock = self.world_data.lock().await;
            lock.spawn_point
        };

        let _ = player.handle.upgrade()?.send(Command::SetLocation {
            location: default_location
        }).await;

        {
            let mut player_lock = self.players.lock();

            player_lock.insert(id, player.clone());
            let player_id = player.id.upgrade()?.load(Ordering::Relaxed);

            for (id, other) in player_lock.iter().map(|(i, p)| (*i, p.clone())) {
                let Some(name) = other.username.upgrade() else { continue; };
                let name = name.lock().clone();

                if let Some(other_handle) = other.handle.upgrade() {
                    let name = player_name.clone();
                    tokio::spawn(async move {
                        let _ = other_handle.send(Command::NotifyJoin {
                            id: player_id,
                            location: default_location,
                            name,
                        }).await;
                    });
                }

                if let Some(other_loc) = other.location.upgrade() {
                    let handle = player.handle.clone();
                    let Some(upgraded) = handle.upgrade() else { continue; };
                    tokio::spawn(async move {
                        let _ = upgraded.send(Command::NotifyJoin {
                            id,
                            location: other_loc.as_ref().into(),
                            name,
                        }).await;
                    });
                }
            }
        }

        Some(id)
    }

    /// Gets a player by their ID.
    #[inline]
    #[must_use]
    pub fn get_player(&self, id: i8) -> Option<WeakPlayer> {
        self.players.lock().get(&id).cloned()
    }

    /// Removes any players with no references left.
    pub fn collect_garbage(&self) {
        let Some(mut lock) = self.players.try_lock() else { return; };
        let mut ids = Vec::new();
        lock.retain(|id, player| {
            if player.any_dropped() {
                ids.push(*id);
                false
            } else { true }
        });
        if !ids.is_empty() {
            debug!("Dropping players {:?}", 
                Itertools::intersperse(
                    ids.iter().map(ToString::to_string), ",".to_string()
                ).collect::<Vec<_>>()
            );
        }
        drop(lock);
        for id in ids {
            self.remove_player(id);
        }
    }

    /// Removes a player from the world.
    /// Returns the player if they exist.
    #[inline]
    pub fn remove_player(&self, id: i8) -> Option<WeakPlayer> {
        {
            let mut lock = self.available_ids.lock();
            lock.push(id);
        }

        let mut player_lock = self.players.lock();

        let removed = player_lock.remove(&id);

        for player in player_lock.values().cloned() {
            tokio::spawn(async move {
                if let Some(handle) = player.handle.upgrade() {
                    let _ = handle.send(Command::NotifyLeave { id }).await;
                }
            });
        }

        drop(player_lock);

        self.collect_garbage();

        removed
    }

    /// Sets a block in the world, notifying all players in the world that it changed.
    ///
    /// This **does not block**, and instead returns a false boolean if the level data is locked.
    pub fn set_block(&self, location: Vector3<u16>, id: u8) -> bool {
        {
            let Ok(mut data_lock) = self.world_data.try_lock() else {
                return false;
            };

            let Some(block) = data_lock.level_data.get_mut(location) else {
                // Placed block out of bounds, this isn't necessarily fatal
                return true;
            };

            *block = id;
        }

        {
            let player_lock = self.players.lock();

            for player in player_lock.values().cloned() {
                tokio::spawn(async move {
                    if let Some(handle) = player.handle.upgrade() {
                        let _ = handle.send(Command::SetBlock {
                            location,
                            id,
                        }).await;
                    }
                }); // No real need to wait for these to send
            }
        }

        true
    }

    /// Move a player in the world, notifying all other players of their movement.
    pub fn move_player(&self, id: i8, location: Location) {
        let player_lock = self.players.lock();

        for player in player_lock.values().cloned() {
            tokio::spawn(async move {
                if let Some(handle) = player.handle.upgrade() {
                    let _ = handle.send(Command::NotifyMove {
                        id,
                        location,
                    }).await;
                }
            }); // No real need to wait for these to send
        }
    }

    /// Constructs a world from a [`WorldData`] and [`PathBuf`].
    pub fn from_data(data: WorldData, path: PathBuf) -> Self {
        Self {
            filepath: Arc::new(path),
            players: Arc::new(Default::default()),
            available_ids: Arc::new(Mutex::new(
                (i8::MIN..=i8::MAX).collect()
            )),
            world_data: Arc::new(TokioMutex::new(data)),
        }
    }
}