#![warn(clippy::pedantic, clippy::perf, missing_docs)]
#![allow(incomplete_features, clippy::doc_markdown)]

//! A simple server software for Classic Minecraft and ClassiCube.

pub mod packets;
pub mod networking;
pub mod world;
pub mod server;