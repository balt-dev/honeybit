//! Handles the actual server.

use std::{
    collections::{HashMap, HashSet}, net::IpAddr, time::Duration
};

use rand::{Rng, rngs::StdRng};

/// A trait to help generate valid salts for the server.
pub trait SaltExt {
    /// Generate a salt.
    fn salt(&mut self) -> String;
}

impl SaltExt for StdRng {
    #[inline]
    fn salt(&mut self) -> String {
        const SALT_MIN: u128 =    768_909_704_948_766_668_552_634_368; // base62::decode("1000000000000000").unwrap();
        const SALT_MAX: u128 = 47_672_401_706_823_533_450_263_330_815; // base62::decode("zzzzzzzzzzzzzzzz").unwrap();
        let num: u128 = self.gen_range(SALT_MIN ..= SALT_MAX);
        base62::encode(num)
    }
}