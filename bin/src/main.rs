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
    let mut config_file = File::open("./config.toml")?;
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

fn set_up_defaults() -> Result<(), Box<dyn Error>> {
    if !Path::new("./config.toml").exists() {
        let mut file = File::create("./config.toml")?;

        let mut buf = String::new();
        Config::default().serialize(toml::Serializer::pretty(&mut buf))?;
        // Insert documentation to the config file
        let comment_map = [
            ("packet_timeout", "How long the server should wait before disconnecting a player, in seconds."),
            ("ping_spacing", "How often the server sends pings to clients, in seconds."),
            ("default_world", "The world that players first connect to when joining."),
            ("operators", "A list of usernames that have operator permissions."),
            ("kept_salts", "How many \"salts\" to keep in memory.\nSalts are used to verify a user's key.\nIf this is set to 0, then users will not be verified."),
            ("name", "The server's displayed name."),
            ("heartbeat_url", "The URL to ping for heartbeat pings.\n\nIf this is left blank, then no heartbeat pings will be sent.\nIf this is left blank AND kept_salts is above 0,\nthe program will exit with an error,\nas it will be impossible for users to join."),
            ("heartbeat_spacing", "How often heartbeat pings will be sent, in seconds."),
            ("heartbeat_timeout", "How long the server will wait to hear back from the heartbeat server, in seconds."),
            ("port", "The port to host the server on."),
            ("max_players", "The maximum amount of players on the server."),
            ("public", "Whether the server will show as public on the heartbeat URLs corresponding server list."),
            ("motd", "The server's MOTD."),
            ("[banned_ips]", "A mapping of IPs to ban reasons."),
            ("[banned_users]", "A mapping of usernames to ban reasons."),
        ];
        
        let mut concat = Vec::new();
        
        for line in buf.lines() {
            let mut commented = false;
            for (prefix, comment) in comment_map {
                if line.starts_with(prefix) {
                    for comment_line in comment.lines() {
                        concat.push("# ");
                        concat.push(comment_line);
                        concat.push("\n");
                    }
                    commented = !prefix.starts_with('[');
                    break;
                }
            }
            concat.push(line);
            concat.push("\n");
            if commented {
                concat.push("\n");
            }
        }
        
        let concatenated = concat.join("");
        
        file.write_all(concatenated.as_bytes())?;
    };
    
    Ok(())
}
