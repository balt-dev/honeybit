#![warn(clippy::pedantic, clippy::perf, missing_docs, clippy::missing_docs_in_private_items)]

//! TODO: This binary implementation is temporary. Don't keep it around.

mod networking;

use std::{
    error::Error,
    process::ExitCode,
    collections::VecDeque
};
use tokio::{
    io::AsyncWriteExt,
    net::{TcpListener, TcpStream}
};
use oxine::{
    networking::IncomingPacketType,
    packets::Incoming
};

#[macro_use]
extern crate log;

#[tokio::main]
async fn main() -> ExitCode {
    simplelog::TermLogger::init(
        if cfg!(debug_assertions) {
            simplelog::LevelFilter::Trace
        } else {
            simplelog::LevelFilter::Info
        },
        simplelog::Config::default(),
        simplelog::TerminalMode::Mixed,
        simplelog::ColorChoice::Auto
    ).expect("no logger has been initialized yet");

    let res = inner_main().await;
    let Err(err) = res else { return ExitCode::SUCCESS; };
    error!("ENCOUNTERED FATAL ERROR");
    error!("{err}");
    ExitCode::FAILURE
}

/// Inner main for better handling of errors
async fn inner_main() -> Result<(), Box<dyn Error>> {
    networking::start().await.map_err(|e| e.into())
}
