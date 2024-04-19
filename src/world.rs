//! Holds structs pertaining to a world in a server.

use std::sync::{Arc};
use std::sync::atomic::Ordering;
use arrayvec::ArrayVec;
use mint::Vector3;
use crate::packets::{AtomicLocation, Location};
use identity_hash::IntMap;
use itertools::Itertools;
use crate::player::{
    WeakPlayer,
    Command
};
use tokio::sync::Mutex as TokioMutex;
use parking_lot::Mutex;
use crate::level_serde::WorldData;


/// A single world within a server.
#[derive(Debug, Clone)]
pub struct World {
    /// The world's name.
    pub name: Arc<Mutex<String>>,
    /// A hashmap of player IDs to players.
    pub players: Arc<Mutex<IntMap<i8, WeakPlayer>>>,
    /// A list of available player IDs.
    pub available_ids: Arc<Mutex<ArrayVec<i8, 256>>>,
    /// The stored level data of the world.
    pub level_data: Arc<TokioMutex<LevelData>>,
    /// A player's default location.
    pub default_location: Arc<AtomicLocation>
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
            debug!("{position:?}, {:?}", self.dimensions);
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

impl Default for World {
    fn default() -> Self {
        Self {
            name: Arc::default(),
            players: Arc::default(),
            available_ids: Arc::new(Mutex::new(
                [0].into_iter().collect()
            )),
            level_data: Arc::new(TokioMutex::new(
                LevelData::new(
                    vec![],
                    Vector3 {x: 0, y: 0, z: 0}
                )
            )),
            default_location: Arc::new(Location {
                position: Vector3 {x: 0.into(), y: 0.into(), z: 0.into()},
                yaw: 0,
                pitch: 0,
            }.into())
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
    #[inline]
    pub async fn add_player(&self, player: WeakPlayer) -> Option<i8> {
        self.collect_garbage();

        debug!("{:?}", self.available_ids.lock());
        
        let id = self.available_ids.lock().pop()?;

        debug!("{:?}", self.available_ids.lock());
        
        let player_name = player.username.upgrade()?.lock().clone();
        
        {
            let world_name = self.name.lock();
            debug!("Adding player {player_name} ({id}) to world {world_name}");
        }
        player.id.upgrade()?.store(id, Ordering::Relaxed);
        {
            let world = player.world.upgrade()?;
            let mut lock = world.lock();
            *lock = self.clone();
        }
        
        let default_location = self.default_location.as_ref().into();
        
        let _ = player.handle.upgrade()?.send(Command::SetLocation {
            location: default_location
        }).await;

        {
            let mut player_lock = self.players.lock();

            player_lock.insert(id, player.clone());
            let player_id = player.id.upgrade()?.load(Ordering::Relaxed);

            for (id, other) in player_lock.iter().map(|(i, p)| (*i, p.clone())) {
                let Some(name) = other.username.upgrade() else { continue };
                let name = name.lock().clone();
                
                if let Some(other_handle) = other.handle.upgrade() {
                    let name = player_name.clone();
                    tokio::spawn(async move {
                        let _ = other_handle.send(Command::NotifyJoin {
                            id: player_id,
                            location: default_location,
                            name
                        }).await;
                    });
                }
                
                if let Some(other_loc) = other.location.upgrade() {
                    let handle = player.handle.clone();
                    let Some(upgraded) = handle.upgrade() else { continue };
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
        let Some(mut lock) = self.players.try_lock() else { return };
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
            debug!("{:?}", lock);
            lock.push(id);
        }
        
        let mut player_lock = self.players.lock();

        let removed = player_lock.remove(&id);
        
        {
            let world_name = self.name.lock();
            let name = removed.clone().and_then(|v| v.username.upgrade())
                .map_or("<dropped>".into(), |arc| arc.lock().to_owned());
            debug!("Removing player {name} ({id}) from world {world_name}");
        }
        
        debug!("{}", player_lock.len());
        
        for player in player_lock.values().cloned() {
            tokio::spawn(async move {
                if let Some(handle) = player.handle.upgrade() {
                    let _ = handle.send(Command::NotifyLeave { id }).await;
                } else {
                    debug!("Player dropped");
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
            let Ok(mut data_lock) = self.level_data.try_lock() else {
                return false
            };

            let Some(block) = data_lock.get_mut(location) else {
                // Placed block out of bounds, this isn't necessarily fatal
                return true;
            };
            
            *block = id;
        }

        {
            let name = self.name.lock();
            debug!("Set block {location:?} to {id:02x} in world {name}");
        }

        {
            let player_lock = self.players.lock();
            
            for player in player_lock.values().cloned() {
                tokio::spawn(async move {
                    if let Some(handle) = player.handle.upgrade() {
                        let _ = handle.send(Command::SetBlock {
                            location,
                            id
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
                        location
                    }).await;
                }
            }); // No real need to wait for these to send
        }
    }
    
    /// Gets serializable [`WorldData`] from this World.
    /// 
    /// This is not an `impl` of `From<&World> for WorldData` due to needing async.
    pub async fn to_data(&self) -> WorldData {
        WorldData {
            name: self.name.lock().to_string(),
            level_data: self.level_data.lock().await.clone(),
            spawn_point: self.default_location.as_ref().into()
        }
    }
}

impl From<WorldData> for World {
    fn from(value: WorldData) -> Self {
        World {
            name: Arc::new(Mutex::new(value.name)),
            players: Arc::default(),
            available_ids: Arc::new(Mutex::new(
                (i8::MIN ..= i8::MAX).collect()
            )),
            level_data: Arc::new(TokioMutex::new(value.level_data)),
            default_location: Arc::new(value.spawn_point.into()),
        }
    }
}