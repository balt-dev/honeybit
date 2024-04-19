#![warn(clippy::pedantic, clippy::perf, missing_docs)]

#![doc = include_str!("../README.md")]

mod network;
mod player;
mod structs;
mod world;
mod read_level;

use std::{
    error::Error,
    fs,
    io,
    process::ExitCode,
    collections::{HashMap, HashSet},
    fs::File,
    io::{ErrorKind, Read, Seek, SeekFrom, Write},
    path::Path,
    time::{Duration}
};
use chrono::Local;
use serde::{Deserialize, Serialize};
use simplelog::{ColorChoice, TerminalMode};
use crate::{
    world::World,
    network::IdleServer,
    structs::Config
};

#[macro_use]
extern crate log;

#[tokio::main]
async fn main() -> ExitCode {
    let now = Local::now();

    match fs::create_dir("./logs") {
        Ok(()) => {},
        Err(err) if err.kind() == ErrorKind::AlreadyExists => {},
        Err(err) => {
            eprintln!("Failed to create log directory: {err}");
            return ExitCode::FAILURE;
        }
    }


    let log_file = match File::create(format!("./logs/{}.log", now.to_rfc3339())) {
        Ok(file) => file,
        Err(err) => {
            eprintln!("Failed to open log file: {err}");
            return ExitCode::FAILURE
        }
    };

    simplelog::CombinedLogger::init(vec![
        simplelog::WriteLogger::new(
            if cfg!(debug_assertions) {
                simplelog::LevelFilter::Trace
            } else {
                simplelog::LevelFilter::Info
            },
            simplelog::ConfigBuilder::default()
                .add_filter_ignore("hyper_util".into())
                .build(),
            log_file
        ),
        simplelog::TermLogger::new(
            simplelog::LevelFilter::Error,
            simplelog::Config::default(),
            TerminalMode::Stderr,
            ColorChoice::Auto
        )
    ]).expect("no logger has been initialized yet");
    
    let res: Result<(), Box<dyn Error>> = inner_main().await.map_err(Into::into);
    let Err(err) = res else { return ExitCode::SUCCESS; };
    error!("~~~ ENCOUNTERED FATAL ERROR ~~~");
    error!("{err}");
    ExitCode::FAILURE
}

#[allow(unreachable_code)] // TODO
/// Inner main function to easily pass back errors
async fn inner_main() -> Result<(), Box<dyn Error>> {
    set_up_defaults()?;
    
    let mut config_string = String::new();
    config_file.read_to_string(&mut config_string)?;
    
    let config = Config::deserialize(toml::Deserializer::new(&config_string))?;
    
    let server: IdleServer = IdleServer {
        worlds: HashMap::from([
            ("debug".into(), World::default()),
            ("debug2".into(), World::default()),
            ("debug3".into(), World::default())
        ]),
        config,
    };
    
    let handle = server.start().await?;
    
    tokio::time::sleep(Duration::MAX).await;
    
    unreachable!("the program should not be running for 500 billion years")
}

fn set_up_defaults() -> io::Result<()> {
    if !Path::new("./config.toml").exists() {
            let mut file = File::create("./config.toml")?;

            let mut buf = String::new();
            Config::default().serialize(toml::Serializer::pretty(&mut buf))?;
            file.write_all(buf.as_bytes())?;
    };
    
    Ok(())
}
