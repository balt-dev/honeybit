//! Handles world generation.

use std::{iter, fmt};

use mint::Vector3;

/// A world generator. 
pub trait WorldGenerator: fmt::Debug + Send + Sync {
    /// Generates level data from world dimensions.
    /// This must return a raw data buffer of world data,
    /// exactly the length of the volume of the dimensions given.
    /// 
    /// This should never fail.
    fn generate(&self, dimensions: Vector3<u16>, seed: u64) -> Vec<u8>;
}

/// Generates a superflat world with the specified layers.
#[derive(Debug)]
pub struct Superflat {
    /// The list of layers in the world.
    pub layers: Vec<(u8, u16)>
}

impl WorldGenerator for Superflat {
    fn generate(&self, dimensions: Vector3<u16>, _seed: u64) -> Vec<u8> {
        let slice_size = dimensions.x as usize * dimensions.y as usize;
        let size = slice_size * dimensions.z as usize;
        let mut buf = Vec::with_capacity(size);
        let mut current_height = 0u16;
        let mut do_break = false;
        for (block, mut height) in self.layers.iter().copied() {
            if !current_height.checked_add(height).is_some_and(|add| add <= dimensions.y) {
                height = current_height - dimensions.y;
                do_break = true;
            }
            current_height += height;
            buf.extend(iter::repeat(block).take(slice_size * height as usize));
            if do_break { break }
        }
        buf
    }
}