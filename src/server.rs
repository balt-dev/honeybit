//! Handles general networking
#![allow(clippy::unnecessary_to_owned)]

// TODO: Refactor this to not be one giant file

use crate::{
    packets::{Outgoing, OutgoingPacketType},
    world::World,
    structs::Config,
    player::{Player, WeakPlayer}
};
use rand::{
    rngs::StdRng,
    SeedableRng,
    Rng
};
use std::{
    collections::{HashMap, VecDeque},
    io::ErrorKind,
    net::Ipv4Addr,
    sync::Arc,
    time::Instant,
    sync::OnceLock
};
use futures::future::join_all;
use tokio::{
    io::{self, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::{mpsc, Mutex as TokioMutex},
    time,
};
use reqwest::StatusCode;
use parking_lot::{Condvar, Mutex};
pub trait SaltExt {
    /// Generate a salt.
    fn salt(&mut self) -> String;
}

impl SaltExt for StdRng {
    #[inline]
    fn salt(&mut self) -> String {
        const SALT_MIN: u128 = 768_909_704_948_766_668_552_634_368; // base62::decode("1000000000000000").unwrap();
        const SALT_MAX: u128 = 47_672_401_706_823_533_450_263_330_815; // base62::decode("zzzzzzzzzzzzzzzz").unwrap();
        let num: u128 = self.gen_range(SALT_MIN..=SALT_MAX);
        base62::encode(num)
    }
}


#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ServerCommand {
    Stop,
    SendChatMessage {
        /// The message to be sent.
        message: String,
    },
}

/// A server that hasn't been started yet.
#[derive(Debug, Clone)]
pub struct IdleServer {
    /// A mapping of names to worlds in the server.
    pub worlds: HashMap<String, World>,
    /// The configuration for the server.
    pub config: Config,
}

macro_rules! command_wrapper {
    ($(
        $(#[$outer:meta])*
        pub async fn $name: ident(&self $(,$($var: ident: $ty: ty),*)?) => $command: ident;
    )+) => {$(
        $(#[$outer])*
        pub async fn $name(&self$(, $($var: impl Into<$ty>),+)?) {
            let _ = self.handle.send(ServerCommand::$command $({ $($var: $var.into()),* })?).await;
        }
    )+};
}

#[derive(Debug, Clone)]
/// A running server. All fields of this are [`Arc<RwLock<_>>`]s, so cloning this will not clone its insides.
/// Think of it like a handle.
pub struct RunningServer {
    /// The worlds in the server.
    pub worlds: Arc<TokioMutex<HashMap<String, World>>>,
    /// The configuration of the server.
    pub config: Arc<Mutex<Config>>,
    /// The default world to send players to.
    pub default_world: World,
    /// A mapping of player names to their info.
    pub connected_players: Arc<TokioMutex<HashMap<String, WeakPlayer>>>,
    /// A list of the last few last salts generated.
    pub last_salts: Arc<Mutex<VecDeque<String>>>,
    /// A handle to send commands to the server.
    pub handle: mpsc::Sender<ServerCommand>,
    /// The server's URL.
    pub url: Arc<OnceLock<String>>,
}

impl RunningServer {
    command_wrapper! {
        /// Sends a message in chat for all players.
        pub async fn send_message(&self, message: String) => SendChatMessage;
        /// Stops the server.
        pub async fn stop(&self) => Stop;
    }

    fn collect_garbage(&self) {
        let Ok(mut lock) = self.connected_players.try_lock() else { return; };
        lock.retain(|_, player| !player.any_dropped());
    }

    fn new(idle: IdleServer, tx: mpsc::Sender<ServerCommand>) -> Option<RunningServer> {
        let default_world = &idle.config.default_world;
        let world = idle.worlds.get(default_world).cloned()?;
        Some(RunningServer {
            worlds: Arc::new(TokioMutex::new(
                idle.worlds
            )),
            default_world: world,
            config: Arc::new(Mutex::new(idle.config)),
            connected_players: Arc::new(TokioMutex::default()),
            last_salts: Arc::new(Mutex::default()),
            handle: tx,
            url: Arc::default(),
        })
    }

    async fn start_commands(self, mut rx: mpsc::Receiver<ServerCommand>, stop_condvar: Arc<Condvar>) {
        while let Some(command) = rx.recv().await {
            match command {
                ServerCommand::SendChatMessage {
                    message
                } => {
                    let lock = self.connected_players.lock().await;
                    // If left with an & prefix, vanilla clients will crash
                    let message = message.strip_suffix('&').unwrap_or(&message);
                    info!("[CHAT] {message}");

                    #[allow(clippy::unnecessary_to_owned)] // False positive
                    for player in lock.values().cloned() {
                        let message = message.to_owned();
                        tokio::spawn(async move {
                            player.send_message(message).await;
                        });
                    }
                }
                ServerCommand::Stop => {
                    info!("Stopping server...");
                    let lock = self.connected_players.lock().await;
                    let mut futures= Vec::new();
                    for player in lock.values().cloned() {
                        futures.push(async move {
                            player.notify_disconnect("Server closed").await;
                        });
                    }
                    join_all(futures).await;
                    rx.close();
                    stop_condvar.notify_one();
                    break;
                }
            }
        }
    }
}

impl IdleServer {
    /// Starts the server. This will immediately return with a handle to send commands to the server.
    ///
    /// # Errors
    /// Errors if the server fails to establish a TCP connection to the configured server port.
    pub async fn start(self, stop_condvar: Arc<Condvar>) -> io::Result<RunningServer> {
        info!("Starting server...");
        let (server_tx, server_rx) =
            mpsc::channel::<ServerCommand>(100);

        let listener = TcpListener::bind((
            Ipv4Addr::new(127, 0, 0, 1),
            self.config.port
        )).await?;
        info!("Connected to port {}", self.config.port);

        let config = self.config.clone();

        let default_world = &config.default_world;

        let Some(server) = RunningServer::new(
            self,
            server_tx.clone(),
        ) else {
            return Err(io::Error::new(ErrorKind::InvalidInput, format!("Default world \"{default_world}\" does not exist")));
        };

        let len = config.heartbeat_url.len();
        if len > 0 {
            let _heartbeat = tokio::spawn(server.clone().start_heartbeat());
        } else if config.kept_salts > 0 {
            return Err(io::Error::new(ErrorKind::InvalidInput, "Cannot verify players if heartbeat URL is unset"));
        }

        let cmd_server = server.clone();

        let _commands = tokio::spawn(cmd_server.start_commands(server_rx, stop_condvar));

        let conn_server = server.clone();

        let _server_task = tokio::spawn(async move {
            let server = conn_server;
            loop {
                server.collect_garbage();
                let connection = listener.accept().await;

                let Ok((mut stream, _)) = connection else {
                    let err = connection.unwrap_err();
                    warn!("TCP connection failed:");
                    warn!("    {err}");
                    continue;
                };

                let Ok(peer) = stream.peer_addr() else {
                    warn!("Failed to get IP of connection");
                    let _ = stream.shutdown().await;
                    continue;
                };

                let peer_ip = peer.ip();

                let ban_reason = {
                    let conf = server.config.lock();
                    conf.banned_ips.get(&peer_ip).map(|reason| format!("Banned: {reason}"))
                };

                if let Some(reason) = ban_reason {
                    info!("Banned IP {peer_ip} attempted to join");
                    let _ = time::timeout(config.packet_timeout,
                        Outgoing::Disconnect { reason }.store(&mut stream),
                    ).await;
                    let _ = stream.shutdown().await;
                    continue;
                }

                debug!("IP {peer_ip} connected");

                tokio::spawn(server.clone().handle_connection(stream));
            }
        });

        Ok(server)
    }
}

#[derive(serde::Deserialize, Clone, Debug)]
struct HeartbeatResponse {
    errors: Vec<Vec<String>>,
    response: String,
    status: String,
}

impl RunningServer {
    /// Starts the heartbeat pings. This will block.
    #[allow(clippy::too_many_lines)]
    async fn start_heartbeat(self) {
        let mut rand = StdRng::from_entropy();
        let http = reqwest::Client::new();

        loop {
            let now = Instant::now();
            let user_count = {
                let lock = self.connected_players.lock().await;
                lock.len()
            };

            // Copy/clone over only the fields we need from the config, so we can drop it ASAP
            // These also can change during runtime, so we fetch them every loop
            let (
                port, max_players, name, public,
                spacing, timeout, url,
                kept_salts
            );
            #[allow(clippy::assigning_clones)]
            {
                let lock = self.config.lock();
                spacing = lock.heartbeat_spacing;
                timeout = lock.heartbeat_timeout;
                url = lock.heartbeat_url.clone();
                max_players = lock.max_players;
                port = lock.port;
                public = lock.public;
                name = lock.name.clone();
                kept_salts = lock.kept_salts;
            }

            let wait_until = now + spacing;

            let salt = {
                let mut lock = self.last_salts.lock();
                if kept_salts == 0 {
                    "0".into() // no salt to be found
                } else {
                    let salt = rand.salt();
                    if lock.len() < kept_salts {
                        // Push a new salt to the front
                        lock.push_front(salt.clone());
                    } else {
                        // Rotate in a new salt, dropping the old one
                        let back = lock.back_mut().unwrap();
                        let _ = std::mem::replace(back, salt.clone());
                        lock.rotate_right(1);
                    }
                    salt
                }
            };

            if !url.is_empty() {
                'b: {
                    let Ok(request) = http.get(&url)
                        .query(&[("port", port)])
                        .query(&[("max", max_players)])
                        .query(&[("name", &name)])
                        .query(&[("public", &public)])
                        .query(&[("version", 7)])
                        .query(&[("salt", salt)])
                        .query(&[("users", user_count)])
                        .query(&[("json", true)])
                        .build()
                    else {
                        warn!("Failed to build heartbeat request, is your configured URL valid?");
                        break 'b;
                    };

                    trace!("Sending heartbeat with URL {}", request.url());

                    let res = time::timeout(timeout, http.execute(request)).await;

                    match res {
                        Ok(Ok(response)) => {
                            let stat = response.status();
                            if StatusCode::OK != stat {
                                warn!(
                                "Got status code {} from heartbeat server{}",
                                stat.as_u16(),
                                stat.canonical_reason().map(|reason| format!(": {reason}")).unwrap_or_default()
                            );
                                break 'b;
                            }

                            let Ok(text) = response.text().await else {
                                warn!("Failed to get text of response to heartbeat ping");
                                break 'b;
                            };

                            let Ok(response): Result<HeartbeatResponse, _> = serde_json::from_str(&text) else {
                                warn!("Failed to decode heartbeat response as JSON: {text}");
                                break 'b;
                            };

                            trace!("Successfully got response of {response:?}");

                            if response.status != "success" {
                                // The ping failed, we warn and stop
                                warn!("Heartbeat ping failed:");
                                for errors in response.errors {
                                    for error in errors {
                                        warn!("\t- {error}");
                                    }
                                }
                                break 'b;
                            }

                            if !response.errors.is_empty() {
                                let length = response.errors.len();
                                warn!("Got {} warning{} from heartbeat:", length, if length > 1 {"s"} else {""});
                                for errors in response.errors {
                                    for error in errors {
                                        warn!("\t- {error}");
                                    }
                                }
                            }

                            let url = response.response;

                            trace!("New url: {url}");

                            self.url.get_or_init(|| url);
                        }
                        Ok(Err(err)) => {
                            warn!("Failed to send heartbeat ping: {err}");
                        }
                        Err(_) => {
                            warn!("Heartbeat server failed to respond in {timeout:?}");
                        }
                    }
                }
            }

            time::sleep_until(wait_until.into()).await;
        }
    }


    /// Handles a TCP connection, consuming it. This will block.
    async fn handle_connection(self, tcp_stream: TcpStream) {
        let (reader, writer) = tcp_stream.into_split();

        let player = Player::new(self.clone(), writer);

        player.downgrade().handle_packets(reader, self.clone()).await;

        // Do this here since garbage collection needs to happen _after_ the player's dropped
        let world = player.world.lock().clone();
        drop(player);
        world.collect_garbage();
        self.collect_garbage();
    }
}
