//! Handles general networking

use {
    crate::player::PlayerCommand, oxine::{
        server::{Config, SaltExt},
        world::World
    }, rand::{
        rngs::StdRng,
        SeedableRng
    }, reqwest::StatusCode, std::{
        collections::{HashMap, VecDeque}, io::ErrorKind,, sync::Arc, net::Ipv4Addr, time::Instant
    }, tokio::{
        io::{self, AsyncWriteExt},
        net::{TcpListener, TcpStream},
        sync::{mpsc, Mutex},
        time
    }
};

use crate::player::Player;

/// Wrapper around locking to easily propagate panics
#[macro_export]
macro_rules! t {
    ($e: expr) => {
        {$e}.expect("other thread panicked")
    };
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ServerCommand {
    Stop,
    SendMessage {
        /// The username of the player that sent the message.
        username: String,
        /// The message to be sent.
        message: String
    }
}

/// A server that hasn't been started yet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdleServer {
    /// A mapping of names to worlds in the server.
    pub worlds: HashMap<String, World>,
    /// The configuration for the server.
    pub config: Config
}

#[derive(Debug, Clone)]
/// A running server. All fields of this are [`Arc<RwLock<_>>`]s, so cloning this will not clone its insides.
/// Think of it like a handle.
pub struct RunningServer {
    /// The worlds in the server.
    pub worlds: Arc<Mutex<HashMap<String, Arc<Mutex<World>>>>>,
    /// The configuration of the server.
    pub config: Arc<Mutex<Config>>,
    /// A mapping of player names to their info.
    pub connected_players: Arc<Mutex<HashMap<String, Player>>>,
    /// A list of the last few last salts generated.
    pub last_salts: Arc<Mutex<VecDeque<String>>>,
    /// A handle to send commands to the server.
    pub commander: Arc<Mutex<mpsc::Sender<ServerCommand>>>,
    /// The server's current URL. Expect this to change often.
    pub url: Arc<Mutex<String>>
}

impl RunningServer {
    fn new(idle: IdleServer, tx: mpsc::Sender<ServerCommand>) -> RunningServer {
        RunningServer {
            worlds: Arc::new(Mutex::new(
                idle.worlds.into_iter().map(
                    |(name, world)| (name, Arc::new(Mutex::new(world)))
                ).collect()
            )),
            config: Arc::new(Mutex::new(idle.config)),
            connected_players: Arc::default(),
            last_salts: Arc::default(),
            commander: Arc::new(Mutex::new(tx)),
            url: Arc::new(Mutex::new(String::new()))
        }
    }
}

impl IdleServer {
    /// Starts the server. This will immediately return with a handle to send commands to the server.
    ///
    /// # Errors
    /// Errors if the server fails to establish a TCP connection to the configured server port.
    pub async fn start(self) -> io::Result<RunningServer> {
        info!("Starting server...");
        let (server_tx, _server_rx) =
            mpsc::channel::<ServerCommand>(100);

        let listener = TcpListener::bind((
            Ipv4Addr::new(127, 0, 0, 1),
            self.config.port
        )).await?;
        info!("Connected to port {}", self.config.port);

        let config = self.config.clone();

        let server = RunningServer::new(
            self,
            server_tx.clone()
        );

        let len = config.heartbeat_url.len();
        if len > 0 {
            let _heartbeat = tokio::spawn(server.clone().start_heartbeat());
        } else if config.kept_salts > 0 {
            return Err(io::Error::new(ErrorKind::InvalidInput, "Cannot verify players if heartbeat URL is unset"))
        }

        let conn_server = server.clone();

        let server_task = tokio::spawn(async move {
            let server = conn_server;
            loop {
                info!("Waiting for connection...");
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

                let banned = {
                    let conf = t!(server.config.lock());
                    conf.banned_ips.contains_key(&peer_ip)
                };

                if banned {
                    info!("Banned IP {peer_ip} attempted to join");
                    let _ = stream.shutdown().await;
                    continue;
                }

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
    status: String
}

impl RunningServer {
    /// Disconnect a player from the server by username.
    /// 
    /// # Errors
    /// Returns an error if the player failed to be notified that it was disconnected.
    pub async fn disconnect(&mut self, username: impl AsRef<str>, reason: impl Into<String>) -> Result<(), mpsc::error::SendError<PlayerCommand>> {
        let mut lock = t!(self.connected_players.lock());
        if let Some(player) = lock.remove(username.as_ref()) {
            if let Some(world_arc) = t!(self.worlds.lock())
                .get(&player.world)
            {
                let mut world_lock = t!(world_arc.lock());
                world_lock.remove_player(player.id);
            }
            player.notify_disconnect(reason).await?;
        }
        Ok(())
    }

    /// Starts the heartbeat pings. This will block.
    #[allow(clippy::too_many_lines)]
    async fn start_heartbeat(self) {
        let mut rand = StdRng::from_entropy();
        let http = reqwest::Client::new();

        loop {
            let now = Instant::now();
            let user_count = {
                let lock = t!(self.connected_players.lock());
                lock.len()
            };

            // Copy/clone over only the fields we need from the config, so we can drop it ASAP
            // These also can change during runtime, so we fetch them every loop
            let (
                port, max_players, name, public,
                spacing, timeout, url,
                kept_salts
            );
            {
                let lock = t!(self.config.lock());
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
                let mut lock = t!(self.last_salts.lock());
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

            'b: {
                let request = match
                    http.get(&url)
                        .query(&[("port", port)])
                        .query(&[("max", max_players)])
                        .query(&[("name", &name)])
                        .query(&[("public", &public)])
                        .query(&[("version", 7)])
                        .query(&[("salt", salt)])
                        .query(&[("users", user_count)])
                        .query(&[("json", true)])
                        .build()
                {
                    Ok(v) => v,
                    Err(err) => {
                        warn!("Failed to build heartbeat URL: {err}");
                        break 'b;
                    }
                };

                debug!("Sending heartbeat with URL {}", request.url());

                let res = time::timeout(timeout, http.execute(request)).await;

                match res {
                    Ok(Ok(response)) =>  {
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
                        
                        debug!("Successfully got response of {response:?}");
                        
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

                        debug!("New url: {url}");
                        *t!(self.url.lock()) = url;
                    },
                    Ok(Err(err)) => {
                        warn!("Failed to send heartbeat ping: {err}");
                    }
                    Err(_) => {
                        warn!("Heartbeat server failed to respond in {timeout:?}");
                    }
                }
            }

            time::sleep_until(wait_until.into()).await;
        }
    }


    /// Handles a TCP connection, consuming it. This will block.
    async fn handle_connection(self, tcp_stream: TcpStream) {
        let (sender, mut reciever) = mpsc::channel::<PlayerCommand>(100);
        let hb_sender = sender.clone();

        let (mut reader, mut writer) = tcp_stream.into_split();
        
        let player = Player::new();

        let timeout = {
            let lock = self.config.lock().expect("other thread panicked");
            lock.packet_timeout
        };

        Player::handle_packets(player, sender, reader, self).await;
    }
}

/*
/// Handle a single connection to the server. This is long-running!
async fn handle_stream(config: Config, server: Arc<RwLock<Server>>, stream: TcpStream) {
    let (tx, mut rx) = mpsc::channel::<Outgoing>(100);
    let htx = tx.clone();
    let (mut read, mut write) = stream.into_split();
    
    let player_name = Arc::new(OnceLock::new());

    let recv_server = server.clone();
    let recv_name = player_name.clone();

    let send_task: JoinHandle<()> = tokio::spawn(async move {
        while !tx.is_closed() {

            let res = Incoming::load(&mut read).await;
            // Using a match instead of .map_err since I need to break
            let res = match res {
                Ok(packet) => packet,
                Err(e) => {
                    let _ = tx.send(Outgoing::Disconnect {
                        reason: format!("Connection error: {e}")
                    }).await;

                    break;
                }
            };
            match res {
                Incoming::PlayerIdentification { version, username, key } => {
                    if version != 0x07 {
                        let _ = tx.send(Outgoing::Disconnect {
                            reason: format!("Failed to connect: Incorrect protocol version 0x{version:02x}")
                        }).await;
                        break;
                    }
                    
                    let verified = {
                        let server = server.read().expect("other thread panicked");

                        server.last_salts.is_empty() || {
                            let mut res = false;
                            for salt in &server.last_salts {
                                let server_key = md5::compute(salt.to_owned() + &username);
                                if *server_key == key.as_bytes() {
                                    res = true;
                                    break;
                                }
                            }
                            res
                        }
                    };
                    
                    if !verified {
                        let _ = tx.send(Outgoing::Disconnect {
                            reason: "Failed to connect: Unauthorized".to_string()
                        }).await;
                        break;
                    }
                    
                    let _ = player_name.set(username);
                }
                Incoming::SetBlock { position, state } => {
                    let lock = server.write().expect("other thread panicked");
                    // Get the world that the player is in
                    let Some((world, id)) = player_name.get().and_then(
                        |name| lock.players_connected.get(name).cloned()
                    ) else { continue /* We aren't ready to recieve these yet */ };
                    
                }
                Incoming::SetLocation { location } => {

                }
                Incoming::Message { message } => {

                }
            }
        }
    });
    
    let heartbeat_task: JoinHandle<()> = tokio::spawn(async move {
        while !htx.is_closed() {
            let next_wakeup = Instant::now() + config.ping_spacing;
            if time::timeout(config.packet_timeout, htx.send(Outgoing::Ping)).await.is_err() {
                // Heartbeat timed out, we disconnect
                let _ = htx.send(
                    Outgoing::Disconnect {reason: "Connection timed out".to_string() }
                ).await;
                break;
            }
            time::sleep_until(next_wakeup).await;
        }
    });

    let (recv, send, heart) = join!(recv_task, send_task, heartbeat_task);
    
    let _ = recv.inspect_err(|err| error!("Recieving task panicked: {err}"));
    let _ = send.inspect_err(|err| error!("Sending task panicked: {err}"));
    let _ = heart.inspect_err(|err| error!("Heartbeat task panicked: {err}"));
}
*/
