//! Handles world generation.

use std::{iter, fmt};
use std::fmt::Display;

use mint::Vector3;

/// A world generator. 
pub trait WorldGenerator: fmt::Debug + Send + Sync {
    /// Generates level data from world dimensions.
    /// This must return a raw data buffer of world data,
    /// exactly the length of the volume of the dimensions given.
    /// 
    /// This should never fail.
    fn generate(&self, dimensions: Vector3<u16>, seed: u64) -> Result<Vec<u8>, String>;
}

/// Generates a superflat world with the specified layers.
#[derive(Debug)]
pub struct Superflat {
    /// The list of layers in the world.
    pub layers: Vec<(u8, u16)>
}

impl WorldGenerator for Superflat {
    fn generate(&self, dimensions: Vector3<u16>, _seed: u64) -> Result<Vec<u8>, String> {
        let slice_size = dimensions.x as usize * dimensions.z as usize;
        let size = slice_size * dimensions.y as usize;
        let mut buf = vec![0; size];
        let mut cursor = 0;
        for (block, height) in self.layers.iter().copied() {
            let part_size = slice_size * height as usize;
            buf[cursor.min(size) .. (cursor + part_size).min(size)].fill(block);
            cursor += part_size;
            if cursor > size {
                buf.truncate(size);
                break;
            }
        }
        Ok(buf)
    }
}
