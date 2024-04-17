#![warn(clippy::pedantic, clippy::perf, missing_docs)]

#![doc = include_str!("../README.md")]

mod network;
mod player;
mod structs;
mod world;

use std::{error::Error, process::ExitCode};
use std::collections::{HashMap, HashSet};
use std::time::Duration;
use crate::world::World;
use crate::network::IdleServer;
use crate::structs::Config;

#[macro_use]
extern crate log;

#[tokio::main]
async fn main() -> ExitCode {
    simplelog::TermLogger::init(
        if cfg!(debug_assertions) {
            simplelog::LevelFilter::Debug
        } else {
            simplelog::LevelFilter::Info
        },
        simplelog::Config::default(),
        simplelog::TerminalMode::Mixed,
        simplelog::ColorChoice::Auto
    ).expect("no logger has been initialized yet");
    
    let res: Result<(), Box<dyn Error>> = inner_main().await.map_err(Into::into);
    let Err(err) = res else { return ExitCode::SUCCESS; };
    error!("~~~ ENCOUNTERED FATAL ERROR ~~~");
    error!("{err}");
    ExitCode::FAILURE
}

#[allow(unreachable_code)] // TODO
/// Inner main function to easily pass back errors
async fn inner_main() -> Result<(), Box<dyn Error>> {
    let server: IdleServer = IdleServer {
        worlds: HashMap::from([
            ("debug".into(), World::default())
        ]),
        config: Config {
            packet_timeout: Duration::from_secs(10),
            ping_spacing: Duration::from_millis(500),
            default_world: "debug".into(),
            banned_ips: HashMap::default(),
            banned_users: HashMap::default(),
            kept_salts: 0,
            name: "OxineTesting".to_string(),
            heartbeat_url: "https://www.classicube.net/server/heartbeat".into(),
            heartbeat_retries: 5,
            heartbeat_spacing: Duration::from_secs(5),
            heartbeat_timeout: Duration::from_secs(5),
            port: 25565,
            max_players: 64,
            public: false,
            operators: HashSet::new(),
            motd: "Oxine MOTD".into(),
        },
    };
    
    let handle = server.start().await?;
    
    tokio::time::sleep(Duration::MAX).await;
    
    unreachable!("the program should not be running for 500 billion years")
}
