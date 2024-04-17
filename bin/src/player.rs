use std::{
    convert,
    io::{self, ErrorKind, Read},
    sync::{atomic::AtomicI8, Arc},
    time::Duration
};
use std::collections::HashMap;
use std::io::{Cursor, Write};
use cfg_if::cfg_if;
use flate2::Compression;
use flate2::read::GzEncoder;
use oxine::{
    networking::{IncomingPacketType as _, OutgoingPacketType},
    packets::{Incoming, Outgoing, Vector3},
    world::Location
};
use tokio::{
    net::tcp::{OwnedReadHalf, OwnedWriteHalf},
    sync::{
        Mutex as TokioMutex,
        mpsc::{self, Sender}
    },
    time
};

use crate::{network::{RunningServer, ServerCommand}, t};

use std::sync::Mutex;

use oxine::world::World;

#[derive(Debug, Clone)]
pub struct Player {
    /// The world the player is in.
    pub world: Arc<Mutex<String>>,
    /// The ID the player has in the world they're in.
    pub id: Arc<AtomicI8>,
    /// A handle to the player's processing loop.
    pub handle: Sender<PlayerCommand>,
    /// The player's username.
    pub username: Arc<Mutex<String>>,
    /// The player's location.
    pub location: Arc<Mutex<Location>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum PlayerCommand {
    Disconnect {
        /// The reason for disconnection.
        reason: String
    },
    /// Initialize a player.
    Initialize {
        username: String
    },
    SendTo { world: String },
}

impl Player {
    /// Notifies the player that it has disconnected from the server.
    pub async fn notify_disconnect(&self, reason: impl Into<String>) -> Result<(), mpsc::error::SendError<PlayerCommand>> {
        self.handle.send(PlayerCommand::Disconnect {
            reason: reason.into()
        }).await
    }

    /// Create a new empty player.
    pub fn new(server: RunningServer, writer: OwnedWriteHalf) -> Player {
        let (tx, rx) = mpsc::channel(128);

        let player = Player {
            world: Arc::new(Mutex::new(String::new())),
            id: Arc::new(AtomicI8::new(-1)),
            handle: tx,
            username: Arc::new(Mutex::new(String::new())),
            location: Arc::new(Mutex::new(Location {
                position: Vector3 { x: 0.into(), y: 0.into(), z: 0.into() },
                yaw: 0,
                pitch: 0,
            })),
        };

        tokio::spawn(player.clone().start_loops(rx, server, writer));

        player
    }

    /// Start the event loop for a player. This will handle all commands recieved over the Receiver, and start a task to periodically send heartbeats.
    pub async fn start_loops(self, mut rx: mpsc::Receiver<PlayerCommand>, server: RunningServer, writer: OwnedWriteHalf) {
        let (packet_timeout, ping_spacing) = {
            let config = t!(server.config.lock());
            (config.packet_timeout, config.ping_spacing)
        };

        let (packet_send, packet_recv) = mpsc::channel(128);

        tokio::spawn(self.clone().start_packets(packet_recv, writer, packet_timeout));
        tokio::spawn(self.clone().start_heartbeat(packet_send.clone(), ping_spacing, packet_timeout));

        while let Some(command) = rx.recv().await {
            match command {
                PlayerCommand::Disconnect { reason } => {
                    let _ = packet_send.send(
                        Outgoing::Disconnect { reason }
                    ).await;
                    rx.close();
                    break;
                }
                PlayerCommand::Initialize { username } => {
                    let (name, motd, operator): (String, String, bool);
                    #[allow(clippy::assigning_clones)] // Doesn't work with non-muts
                    {
                        let lock = t!(server.config.lock());
                        name = lock.name.clone();
                        motd = lock.motd.clone();
                        operator = lock.operators.contains(&username);
                    };
                    let res = packet_send.send(Outgoing::ServerIdentification {
                        version: 7,
                        name,
                        motd,
                        operator,
                    }
                    ).await;
                    if let Err(e) = res {
                        let _ = self.notify_disconnect(format!("Connection failed: {e}")).await;
                    }
                }
                PlayerCommand::SendTo { world } => {
                    self.send_to(world, server.worlds.clone(), packet_send.clone()).await;
                }
            }
        }
    }

    /// Start the loop for sending packets to the client.
    async fn start_packets(self, mut recv: mpsc::Receiver<Outgoing>, mut writer: OwnedWriteHalf, timeout: Duration) {
        while let Some(packet) = recv.recv().await {
            let Ok(()) = time::timeout(timeout, packet.store(&mut writer))
                .await
                .map_err(|_| io::Error::from(ErrorKind::TimedOut))
                .and_then(convert::identity) // Flatten error (.flatten() is not stable yet)
                else { break; };
            debug!("Sent packet {packet:?}");
        }
    }

    /// Start the heartbeat loop for a player.
    async fn start_heartbeat(self, send: Sender<Outgoing>, spacing: Duration, timeout: Duration) {
        let mut interval = time::interval(spacing);

        while time::timeout(timeout, send.send(Outgoing::Ping))
            .await
            .map_err(|_| ())
            .map(|v| v.map_err(|_| ()))
            .and_then(convert::identity)
            .is_ok()
        {
            interval.tick().await;
        }

        // We timed out, shut off everything
        let _ = self.notify_disconnect("Timed out").await;
    }

    /// Handle the packets for a player. This will block.
    pub async fn handle_packets(self, mut stream: OwnedReadHalf, server: RunningServer) {
        let verify = {
            let lock = server.config.lock().expect("other thread panicked");
            lock.kept_salts != 0
        };

        while !self.handle.is_closed() {

            // Using a match instead of .map_err since I need to break
            let res = match Incoming::load(&mut stream).await {
                Ok(v) => v,
                Err(err) => {
                    let _ = self.notify_disconnect(format!("Connection died: {err}")).await;
                    break;
                }
            };

            debug!("Received packet {res:?}");

            // Actually handle the packet
            match res {
                Incoming::PlayerIdentification { version, username, key } => {
                    if version != 0x07 {
                        let _ = self.handle.send(PlayerCommand::Disconnect {
                            reason: format!("Failed to connect: Incorrect protocol version {version}")
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
                        username
                    }).await else { break; };
                }

                Incoming::SetBlock { position, state } => {}

                Incoming::SetLocation { location } => {}

                Incoming::Message { message } => {
                    let username = {
                        let lock = t!(self.username.lock());
                        lock.clone()
                    };

                    let res = {
                        server.commander.send(ServerCommand::SendMessage {
                            username,
                            message,
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

    /// Send the player to a world.
    async fn send_to(&self, world_name: impl Into<String>, worlds_arc: Arc<TokioMutex<HashMap<String, Arc<TokioMutex<World>>>>>, send: Sender<Outgoing>) {
        let world_name = world_name.into();

        {
            let mut lock = t!(self.world.lock());
            cfg_if! {
                if #[cfg(debug_assertions)] {
                    (*lock).clone_from(&world_name);
                } else {
                    *lock = world_name.clone();
                }
            }
        }


        // The future checker doesn't consider dropping a mutex lock
        // via std::mem::drop
        // as making it unusable, so we do this instead.
        let disconnect_reason = 'b: {

            let worlds_lock = worlds_arc.lock().await;
            debug!("Connecting to world {world_name}");
            let Some(world) = worlds_lock.get(&world_name) else {
                break 'b format!("Sent to nonexistent world {world_name}");
            };
            let mut lock = world.lock().await;

            let dimensions = lock.level_data.dimensions;

            let Some(player_id) = lock.create_player() else {
                break 'b format!("World {world_name} is full");
            };

            let id = player_id;

            // GZip level data
            let data_slice = lock.level_data.raw_data.as_slice();

            debug!("{} bytes to compress", data_slice.len());

            if let Err(err) = send.send(Outgoing::LevelInit).await {
                break 'b format!("IO error: {err}")
            }
            
            let mut encoder = GzEncoder::new(WorldEncoder::new(data_slice), Compression::fast());
            
            // For some reason, streaming the encoded data refused to work.
            // Really annoying but oh well I guess >:/
            let mut encoded_data = Vec::new();
            encoder.read_to_end(&mut encoded_data).expect("reading into Vec should never fail");
            
            let mut buf = [0; 1024];
            
            let iter = encoded_data.chunks(1024);
            let chunk_count = iter.len();

            for (i, chunk) in iter.enumerate() {
                buf[..chunk.len()].copy_from_slice(chunk);
                
                #[allow(clippy::pedantic)]
                if let Err(err) = send.send(Outgoing::LevelDataChunk {
                    data_length: chunk.len() as u16,
                    data_chunk: Box::new(buf),
                    percent_complete: ((i as f32) / (chunk_count as f32) * 100.0) as u8
                }).await {
                    break 'b format!("IO error: {err}")
                }
            }

            if let Err(err) = send.send(Outgoing::LevelFinalize {
                size: dimensions
            }).await {
                break 'b format!("IO error: {err}")
            }

            return;
        };
        let _ = self.notify_disconnect(format!("World {world_name} is full")).await;
    }
}


struct WorldEncoder<'inner> {
    inner: Cursor<&'inner [u8]>,
    length_read: bool
}

impl<'inner> WorldEncoder<'inner> {
    fn new(slice: &'inner [u8]) -> Self {
        Self {
            inner: Cursor::new(slice),
            length_read: false
        }
    }
}

impl<'inner> Read for WorldEncoder<'inner> {
    #[allow(clippy::cast_possible_truncation)]
    fn read(&mut self, mut buf: &mut [u8]) -> io::Result<usize> {
        if self.length_read {
            self.inner.read(buf)
        } else {
            let len = self.inner.get_ref().len() as u32;
            let slice = len.to_be_bytes();
            buf.write_all(&slice)?;
            self.length_read = true;
            Ok(4)
        }
    }
}
