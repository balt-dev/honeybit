#![warn(clippy::pedantic, clippy::perf, missing_docs, clippy::missing_docs_in_private_items)]
#![allow(incomplete_features, clippy::doc_markdown)]

//! A simple server software for Classic Minecraft and ClassiCube.

pub mod packets;
pub mod networking;
pub mod world;
pub mod server;