#![feature(str_split_whitespace_remainder)]
#![warn(clippy::pedantic, clippy::perf, missing_docs)]

#![doc = include_str!("../README.md")]

mod server;
mod player;
mod structs;
mod world;
mod level_serde;
mod packets;
mod worldgen;

use std::{
    error::Error,
    fs,
    process::ExitCode,
    collections::HashMap,
    fs::File,
    io::{ErrorKind, Read, Write},
    path::Path,
    ffi::OsStr,
    sync::Arc,
};
use std::path::PathBuf;
use std::sync::OnceLock;
use chrono::Local;
use serde::Deserialize;
use simplelog::{ColorChoice, TerminalMode};
use crate::{
    world::{WorldData, World},
    server::IdleServer,
    structs::Config,
};
use dirs::data_local_dir;
use parking_lot::{Condvar, Mutex};

#[macro_use]
extern crate log;

#[tokio::main]
async fn main() -> ExitCode {
    let path = if cfg!(debug_assertions) {
        let Ok(path) = std::env::current_dir() else {
            eprintln!("Failed to get current path");
            return ExitCode::FAILURE;
        };
        path.join("data")
    } else if let Some(p) = data_local_dir() {
        p.join("honeybit")
    } else {
        let Ok(path) = std::env::current_exe() else {
            eprintln!("Failed to get current path");
            return ExitCode::FAILURE;
        };
        path.parent().expect("executable path always has a parent").to_path_buf()
    };

    let new_local_data = !path.exists();
    
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

    let log_name = now.to_rfc3339().replace(":", "_");

    let log_path = logs_path.join(format!("{}.log", log_name));
    
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
            if cfg!(debug_assertions) {
                simplelog::LevelFilter::Debug
            } else {
                simplelog::LevelFilter::Info
            },
            simplelog::Config::default(),
            TerminalMode::Stderr,
            ColorChoice::Auto
        )
    ]).expect("no logger has been initialized yet");

    if new_local_data {
        info!("Created new local directory at {}. Check your config file!", path.display());
    }

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
        Err(format!($msg, $($($fmt),+,)? $err ))?;
        unreachable!()
    };
    (;warn $msg: literal $err: ident $($($fmt: expr),+)?) => {
        warn!($msg, $($($fmt),+,)? $err );
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

    if config.heartbeat_url.is_empty() {
        info!("Heartbeat URL is empty, not connecting.");
    } else if config.kept_salts == 0 && config.public {
        return Err("You are not verifying users AND publicly hosting the server, allowing anyone to log in as an operator or bypass bans. Refusing to start.".into())
    }

    let worlds = load_worlds(path).await?;
    
    let server: IdleServer = IdleServer {
        worlds,
        config,
    };

    let stop_notifier = Arc::new(Condvar::new());
    let handle = try_with_context!(
        server.start(stop_notifier.clone()).await;
        error "Startup: {}"
    );

    let stop_handle = handle.clone();
    if let Err(err) = ctrlc_async::set_async_handler(async move {
        stop_handle.stop().await;
    }) {
        warn!("Failed to set CTRL-C handler: {err}");
        warn!("Server will not gracefully shut down unless you do /stop.");
    }

    {
        let mutex = Mutex::new(());
        let mut lock = mutex.lock();
        stop_notifier.wait(&mut lock);
    }

    // Save the server's worlds
    {
        let worlds = handle.worlds.lock().await;
        for (name, world) in worlds.iter().map(|(name, world)| (name, world.clone())) {
            let Err(err) = world.save().await else {
                info!("Saved world {name}");
                continue;
            };
            warn!("Failed to save world {name}: {err}");
        }
    }

    // Save the config
    {
        let config = handle.config.lock();
        let mut buf = String::new();
        if let Err(err) = config.save(&mut buf).and_then(|_| {
            let config_path = path.join("config.toml");
            let backup_path = config_path.with_extension("toml~");
            fs::rename(&config_path, backup_path)?;
            let mut file = File::create(config_path)?;
            file.write_all(buf.as_bytes())
        }) {
            warn!("Failed to save config: {err}");
            warn!("To mitigate data loss, config will be dumped to console.");
            warn!("Current config: {config:?}");
        }
    }

    Ok(())
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
    
    debug!("{config:?}");
    Ok(config)
}

static WORLD_PATH: OnceLock<PathBuf> = OnceLock::new();

async fn load_worlds(path: &Path) -> Result<HashMap<String, World>, Box<dyn Error>> {
    let world_dir = path.join("worlds");
    WORLD_PATH.get_or_init(|| world_dir.clone());
    
    let worlds = try_with_context!(
        fs::read_dir(world_dir);
        error "Failed to open worlds directory: {}"
    );
    
    let mut world_map: HashMap<String, World> = HashMap::new();
    
    for world in worlds {
        let world = try_with_context!(world; error "Failed to read worlds directory: {}");
        let mut path = world.path();

        // For windows users
        if path.file_name() == Some(OsStr::new("desktop.ini"))
            // Ignore backups
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
                File::create(&path);
                warn "Failed to create {}: {}"; path.display()
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

        let world = World::from_data(world_data, Some(path.clone()));
        let mut name = {
            let lock = world.data.lock().await;
            lock.name.clone()
        };

        if let Some(occupied) = world_map.get(&name) {
            warn!("Two worlds have the same name of {name}:");
            warn!("- {}", path.display());
            warn!("- {}", occupied.filepath.as_ref().get().expect("worlds loaded from files always have a name").display());
            warn!("Renaming {}...", path.display());
            let mut counter = 0;
            let mut new_name = name.clone() + " (1)";
            while world_map.contains_key(&new_name) {
                counter += 1;
                let new_suf = format!(" ({counter})");
                new_name = name.clone() + &new_suf;
            }
            warn!("Renamed to {new_name}");
            name = new_name;
            {
                let mut lock = occupied.data.lock().await;
                lock.name.clone_from(&name);
                let backup_path = path.with_extension("hbit~");
                try_with_context!(
                    fs::copy(&path, backup_path);
                    warn "Failed to copy {}: {}"; path.display()
                );
                let mut file = try_with_context!(
                    File::create(&path);
                    warn "Failed to open {}: {}"; path.display()
                );
                try_with_context!(
                    lock.store(&mut file);
                    warn "Failed to save to {}: {}"; path.display()
                );
            }
        }

        info!("Loaded world \"{name}\" from {}", path.display());

        world_map.insert(name, world);
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

static DEFAULT_WORLD: &[u8] = include_bytes!("default.hbit");

fn make_worlds(path: &Path) -> Result<(), Box<dyn Error>> {
    let world_dir = path.join("worlds");
    if !world_dir.exists() {
        // Create world directory
        try_with_context!(
            fs::create_dir(&world_dir);
            error "Creating worlds directory: {}"
        );
        // Load default world into it
        let default_path = world_dir.join("default.hbit");
        let mut file = try_with_context!(
            File::create(default_path);
            error "Creating default world file: {}"
        );
        try_with_context!(
            file.write_all(DEFAULT_WORLD);
            error "Writing default world: {}"
        );
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
            Config::default().save(&mut buf);
            error "Serializing default configuration: {}"
        );

        try_with_context!(
            file.write_all(buf.as_bytes());
            error "Writing default configuration: {}"
        );
    };
    Ok(())
}
