use std::{convert, io::{self, ErrorKind}, sync::{atomic::AtomicI8, Arc, Mutex, TryLockError}};

use oxine::{networking::{IncomingPacketType as _, OutgoingPacketType}, packets::{Incoming, Outgoing, Vector3}, world::Location};
use tokio::{net::{tcp::{OwnedReadHalf, OwnedWriteHalf}, TcpStream}, sync::mpsc::{self, Sender}, time};

use crate::{networking::{RunningServer, ServerCommand}, t};



#[derive(Debug, Clone)]
pub struct Player {
    /// The world the player is in.
    pub world: Arc<Mutex<String>>,
    /// The ID the player has in the world they're in.
    pub id: Arc<AtomicI8>,
    /// A handle to the player's processing loop.
    pub handle: mpsc::Sender<PlayerCommand>,
    /// The player's username.
    pub username: Arc<Mutex<String>>,
    /// The player's location.
    pub location: Arc<Mutex<Location>>
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PlayerCommand {
    Disconnect {
        /// The reason for disconnection.
        reason: String
    },
    /// Initialize a player.
    Initialize {
        username: String,
        world: String
    }
}

impl Player {
    /// Notifies the player that it has disconnected from the server.
    pub async fn notify_disconnect(&self, reason: impl Into<String>) -> Result<(), mpsc::error::SendError<PlayerCommand>> {
        self.handle.send(PlayerCommand::Disconnect {
            reason: reason.into()
        }).await
    }

    /// Create a new empty player.
    pub fn new(server: RunningServer) -> Player {
        let (tx, mut rx) = mpsc::channel(128);
        
        let player = Player {
            world: Arc::new(Mutex::new(String::new())),
            id: Arc::new(AtomicI8::new(-1)),
            handle: tx,
            username: Arc::new(Mutex::new(String::new())),
            location: Arc::new(Mutex::new(Location {
                position: Vector3 { x: 0.into(), y: 0.into(), z: 0.into() },
                yaw: 0,
                pitch: 0
            }))
        };

        tokio::spawn(player.clone().start_loop(rx, server));

        player
    }

    /// Start the event loop for a player. This will handle all commands recieved over the Receiver, and start a task to periodically send heartbeats.
    pub async fn start_loop(self, mut rx: mpsc::Receiver<PlayerCommand>, server: RunningServer) {
        
        let (packet_timeout, ping_spacing) = {
            let config = t!(server.config.lock());
            (config.packet_timeout, config.ping_spacing)
        };

        tokio::spawn(self.clone().start_heartbeat(stream.clone(), packet_timeout, ping_spacing));
        
        while let Some(command) = rx.recv().await {
            match command {
                PlayerCommand::Disconnect { reason } => {
                    let stream_lock = t!(stream.lock());
                    let _ = server.send_packet(
                        Outgoing::Disconnect { reason }
                        .store(stream_lock)
                    ).await;
                    break;
                },
                PlayerCommand::Initialize { username, world } => {
                    let (name, motd, operator);
                    {
                        let lock = t!(server.config.lock());
                        name = lock.name.clone();
                        motd = lock.motd.clone();
                        operator = lock.operators.contains(&username);
                    };
                    let res = server.send_packet(Outgoing::ServerIdentification {
                            version: 7,
                            name,
                            motd,
                            operator
                        }
                    )
                        .await
                        .map_err(|_| io::Error::from(ErrorKind::TimedOut))
                        .and_then(convert::identity); // Flatten error (.flatten() is not stable yet)
                    if let Err(e) = res {
                        let _ = self.notify_disconnect(format!("Connection failed: {e}")).await;
                    }
                }
            }
        }
    }

    /// Start the heartbeat loop for a player.
    async fn start_heartbeat(self, mut stream: OwnedWriteHalf) {
        
    }

    /// Handle the packets for a player. This will block.
    pub async fn handle_packets(self, mut stream: OwnedReadHalf, server: RunningServer) {

        let (verify, timeout, default_world) = {
            let lock = server.config.lock().expect("other thread panicked");
            (lock.kept_salts != 0, lock.packet_timeout, lock.default_world.clone())
        };

        while !self.handle.is_closed() {

            let res = time::timeout(
                timeout, Incoming::load(&mut stream)
            )
                .await
                .map_err(|_| io::Error::from(ErrorKind::TimedOut))
                .and_then(convert::identity); // Flatten error (.flatten() is not stable yet)

            // Using a match instead of .map_err since I need to break
            let res = match res {
                Ok(packet) => packet,
                Err(_) => {
                    let _ = self.notify_disconnect("Could not deserialize packet").await;

                    break;
                }
            };

            // Actually handle the packet
            match res {
                Incoming::PlayerIdentification { version, username, key } => {
                    if version != 0x07 {
                        let _ = self.handle.send(PlayerCommand::Disconnect {
                            reason: format!("Failed to connect: Incorrect protocol version 0x{version:02x}")
                        }).await;
                        break;
                    }
                    
                    let verified = !verify || {
                        let salts = server.last_salts.lock().expect("other thread panicked");

                        let mut res = false;
                        for salt in salts.iter() {
                            let server_key = md5::compute(salt.to_owned() + &username);
                            if *server_key == key.as_bytes() {
                                res = true;
                                break;
                            }
                        }
                        res
                    };
                    
                    if !verified {
                        let _ = self.handle.send(PlayerCommand::Disconnect {
                            reason: "Failed to connect: Unauthorized".to_string()
                        }).await;
                        break;
                    }
                    
                    let Ok(()) = self.handle.send(PlayerCommand::Initialize {
                        username,
                        world: default_world.clone()
                    }).await else { break };
                }

                Incoming::SetBlock { position, state } => {
                    
                    
                }

                Incoming::SetLocation { location } => {

                }

                Incoming::Message { message } => {
                    let username = {
                        let lock = t!(self.username.lock());
                        lock.clone()
                    };

                    let res = {
                        server.commander.send(ServerCommand::SendMessage {
                            username,
                            message
                        }).await
                    };

                    if res.is_err() {
                        let _ = self.handle.send(PlayerCommand::Disconnect { reason: "Server loop died".into() }).await;
                        break;
                    }
                }
            }
        }
    }
}