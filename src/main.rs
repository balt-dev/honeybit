#![warn(clippy::pedantic, clippy::perf, missing_docs)]

#![doc = include_str!("../README.md")]

mod network;
mod player;
mod structs;
mod world;
mod level_serde;
mod packets;

use std::{
    error::Error,
    fs,
    process::ExitCode,
    collections::HashMap,
    fs::File,
    io::{ErrorKind, Read, Write},
    path::Path,
    time::{Duration},
    ffi::OsStr
};
use chrono::Local;
use serde::{Deserialize, Serialize};
use simplelog::{ColorChoice, TerminalMode};
use crate::{
    world::{WorldData, World},
    network::IdleServer,
    structs::Config,
};
use dirs::data_local_dir;

#[macro_use]
extern crate log;

#[tokio::main]
async fn main() -> ExitCode {
    let path = if let Some(p) = data_local_dir() {
        p.join("honeybit")
    } else {
        eprintln!("No local directory found for this OS, using current directory...");
        let Ok(path) = std::env::current_exe() else {
            eprintln!("Failed to get current path");
            return ExitCode::FAILURE;
        };
        path.parent().expect("executable path always has a parent").to_path_buf()
    };
    
    let now = Local::now();

    let logs_path = path.join("logs");
    
    match fs::create_dir_all(&logs_path) {
        Ok(()) => {},
        Err(err) if err.kind() == ErrorKind::AlreadyExists => {},
        Err(err) => {
            eprintln!("Failed to create log directory at {}: {err}", logs_path.display());
            return ExitCode::FAILURE;
        }
    }

    let log_path = logs_path.join(format!("{}.log", now.to_rfc3339()));
    
    let log_file = match File::create(&log_path) {
        Ok(file) => file,
        Err(err) => {
            eprintln!("Failed to open log file at {}: {err}", log_path.display());
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
    
    let res: Result<(), Box<dyn Error>> = inner_main(&path).await.map_err(Into::into);
    let Err(err) = res else { return ExitCode::SUCCESS; };
    error!("~~~ ENCOUNTERED FATAL ERROR ~~~");
    error!("{err}");
    ExitCode::FAILURE
}

macro_rules! try_with_context {
    ($err: expr; $t: ident $msg: literal $(; $($fmt: expr),+)?) => {
        match {$err} {
            Ok(v) => v,
            Err(ref err) => {
                try_with_context!(;$t $msg err $($($fmt),+)?);
            }
        }
    };
    (;error $msg: literal $err: ident $($($fmt: expr),+)?) => {
        Err(format!($msg, $err $(,$($fmt),+)?))?;
        unreachable!()
    };
    (;warn $msg: literal $err: ident $($($fmt: expr),+)?) => {
        warn!($msg, $err $(, $($fmt),+)?);
        continue;
    }
}

/// Inner main function to easily pass back errors
async fn inner_main(path: &Path) -> Result<(), Box<dyn Error>> {
    try_with_context!(
        set_up_defaults(path);
        error "Setting up defaults: {}"
    );

    let config = load_config(path)?;

    let worlds = load_worlds(path).await?;
    
    let server: IdleServer = IdleServer {
        worlds,
        config,
    };
    
    let handle = try_with_context!(
        server.start().await;
        error "Startup: {}"
    );

    
    
    tokio::time::sleep(Duration::MAX).await;
    
    unreachable!("the program should not be running for 500 billion years")
}

fn load_config(path: &Path) -> Result<Config, Box<dyn Error>> {
    let config_path = path.join("config.toml");

    let mut config_string = String::new();
    let mut config_file = try_with_context!(
        File::open(&config_path);
        error "Opening config file: {}"
    );
    try_with_context!(
        config_file.read_to_string(&mut config_string);
        error "Reading config file: {}"
    );

    let mut config = try_with_context!(
        Config::deserialize(toml::Deserializer::new(&config_string));
        error "Deserializing config file: {}"
    );
    config.path = config_path;
    Ok(config)
}

async fn load_worlds(path: &Path) -> Result<HashMap<String, World>, Box<dyn Error>> {
    let world_dir = path.join("worlds");
    
    let worlds = try_with_context!(
        fs::read_dir(world_dir);
        error "Failed to open worlds directory: {}"
    );
    
    let mut world_map = HashMap::new();
    
    for world in worlds {
        let world = try_with_context!(world; error "Failed to read worlds directory: {}");
        let mut path = world.path();

        // For windows users
        if path.file_name() == Some(OsStr::new("desktop.ini")) 
            || path.extension().is_some_and(|ext| ext.as_encoded_bytes().ends_with(b"~"))
        {
            continue
        }

        let file = try_with_context!(
            File::open(&path);
            warn "Failed to open {}: {}"; path.display()
        );

        let (world_data, is_hbit) = try_with_context!(
            WorldData::guess_load(file); 
            warn "Failed to parse {}: {}"; path.display()
        );
        
        if !is_hbit {
            let old_path = path.clone();
            path.set_extension("hbit");
            let file = try_with_context!(
                File::open(&path);
                warn "Failed to open {}: {}"; path.display()
            );
            try_with_context!(
                world_data.store(file);
                warn "Failed to resave {}: {}"; path.display()
            );
            // Now that everything succeeded, rename the old path so we don't trip on it
            let mut ext = old_path.extension().unwrap_or_default().to_owned();
            ext.push("~");
            let new_path = old_path.with_extension(ext);
            if let Err(err) = fs::rename(&old_path, new_path) {
                warn!("Failed to rename old world {}: {err}\nYou need to do this manually to prevent the file from doubling!", old_path.display());
            }
        }

        let world = World::from_data(world_data, path.clone());
        let name = {
            let lock = world.world_data.lock().await;
            lock.name.clone()
        };

        if let Some(occupied) = world_map.insert(name.clone(), world) {
            warn!(
                "Two worlds have the same name of {name}:\n- {}\n- {}",
                path.display(),
                occupied.filepath.display()
            );
        }
    }
    
    Ok(world_map)
}

fn set_up_defaults(path: &Path) -> Result<(), Box<dyn Error>> {

    // Set up default configuration file
    make_config(path)?;

    // Set up world directory
    make_worlds(path)?;

    Ok(())
}

fn make_worlds(path: &Path) -> Result<(), Box<dyn Error>> {
    let world_dir = path.join("worlds");
    if !world_dir.exists() {
        try_with_context!(
            fs::create_dir(world_dir);
            error "Creating worlds directory: {}"
        );
        // Load default world into it
    }
    Ok(())
}

fn make_config(path: &Path) -> Result<(), Box<dyn Error>> {
    let config_path = path.join("config.toml");

    if !config_path.exists() {
        let mut file = try_with_context!(
            File::create(config_path);
            error "Creating config file: {}"
        );

        let mut buf = String::new();
        try_with_context!(
            Config::default().serialize(toml::Serializer::pretty(&mut buf));
            error "Serializing default configuration: {}"
        );
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

        try_with_context!(
            file.write_all(concatenated.as_bytes());
            error "Writing default configuration: {}"
        );
    };
    Ok(())
}
