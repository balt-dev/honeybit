use std::{convert, io::{self, ErrorKind}, sync::{Arc, Mutex, TryLockError}};

use oxine::{networking::IncomingPacketType as _, packets::{Incoming, Vector3}, world::Location};
use tokio::{net::{tcp::OwnedReadHalf, TcpStream}, sync::mpsc, time};

use crate::{networking::{RunningServer, ServerCommand}, t};



#[derive(Debug, Clone)]
pub struct Player {
    /// The world the player is in.
    pub world: String,
    /// The ID the player has in the world they're in.
    pub id: i8,
    /// A handle to the player's processing loop.
    pub handle: mpsc::Sender<PlayerCommand>,
    /// The player's username.
    pub username: String,
    /// The player's location.
    pub location: Location
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
    pub fn new() -> Arc<Mutex<Player>> {
        let (tx, mut rx) = mpsc::channel(128);
        
        let player = Arc::new(Mutex::new(Player {
            world: String::new(),
            id: -1,
            handle: tx,
            username: String::new(),
            location: Location {
                position: Vector3 { x: 0.into(), y: 0.into(), z: 0.into() },
                yaw: 0,
                pitch: 0
            }
        }));

        tokio::spawn(Player::start_loop(player.clone(), rx));

        player
    }

    /// Start the event loop for a player. This will handle all commands recieved over the Receiver.
    pub async fn start_loop(mut player: Arc<Mutex<Player>>, rx: mpsc::Receiver<PlayerCommand>) {
        
    }

    /// Handle the packets for a player. This will block.
    pub async fn handle_packets(mut player: Arc<Mutex<Player>>, tx: mpsc::Sender<PlayerCommand>, mut stream: OwnedReadHalf, server: RunningServer) {

        let (verify, timeout, default_world) = {
            let lock = server.config.lock().expect("other thread panicked");
            (lock.kept_salts != 0, lock.packet_timeout, lock.default_world.clone())
        };

        while !tx.is_closed() {

            let res = time::timeout(
                timeout, Incoming::load(&mut stream)
            )
                .await
                .map_err(|_| io::Error::from(ErrorKind::TimedOut))
                .and_then(convert::identity); // Flatten error (.flatten() is not stable yet)

            // Using a match instead of .map_err since I need to break
            let res = match res {
                Ok(packet) => packet,
                Err(e) => {
                    let name = {
                        match player.try_lock() {
                            Ok(p) => p.username.clone(),
                            Err(TryLockError::WouldBlock) => "<mutex locked>".into(),
                            Err(TryLockError::Poisoned(_)) => panic!("other thread panicked")
                        }
                    };
                    info!("Disconnected player {name} due to connection error");
                    let _ = tx.send(PlayerCommand::Disconnect {
                        reason: format!("Connection error: {e}")
                    }).await;

                    break;
                }
            };

            // Actually handle the packet
            match res {
                Incoming::PlayerIdentification { version, username, key } => {
                    if version != 0x07 {
                        let _ = tx.send(PlayerCommand::Disconnect {
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
                        let _ = tx.send(PlayerCommand::Disconnect {
                            reason: "Failed to connect: Unauthorized".to_string()
                        }).await;
                        break;
                    }
                    
                    let Ok(()) = tx.send(PlayerCommand::Initialize {
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
                        let lock = t!(player.lock());
                        lock.username.clone()
                    };

                    let _ = {
                        let lock = t!(server.commander.lock());

                        lock.send(ServerCommand::SendMessage {
                            username, message
                        }).await
                    };
                }
            }
        }
    }
}