use std::{
    convert,
    io::{self, ErrorKind, Read},
    sync::{
        Arc,
        atomic::AtomicI8,
        atomic::Ordering,
    },
    time::Duration,
    io::{Cursor, Write},
};
use std::backtrace::Backtrace;
use std::sync::atomic::AtomicBool;
use std::sync::Weak;
use pollster::FutureExt as _;
use flate2::{
    Compression,
    read::GzEncoder
};
use oxine::{
    networking::{IncomingPacketType as _, OutgoingPacketType},
    packets::{Incoming, Outgoing, Vector3},
    packets::Location
};
use tokio::{
    net::tcp::{OwnedReadHalf, OwnedWriteHalf},
    sync::{
        mpsc::{self, Sender}
    },
    time
};
use tokio::sync::mpsc::WeakSender;
use oxine::packets::AtomicLocation;
use uuid::Uuid;
use parking_lot::Mutex;

use crate::{
    network::{RunningServer, ServerCommand},
    world::World
};

#[derive(Debug)]
pub struct Player {
    /// The world the player is in.
    pub world: Arc<Mutex<World>>,
    /// The ID the player has in the world they're in.
    pub id: Arc<AtomicI8>,
    /// A handle to the player's processing loop.
    pub handle: Sender<Command>,
    /// A handle to the player's block queue.
    pub block_handle: Sender<(Vector3<u16>, u8)>,
    /// The player's username.
    pub username: Arc<Mutex<String>>,
    /// The player's location.
    pub location: Arc<AtomicLocation>,
    /// Whether the player is connected.
    pub connected: Arc<AtomicBool>,
    /// The player's UUID. This is mainly used for logging.
    pub uuid: Uuid
}

#[derive(Debug, Clone)]
pub(crate) struct WeakPlayer {
    /// The world the player is in.
    pub world: Weak<Mutex<World>>,
    /// The ID the player has in the world they're in.
    pub id: Weak<AtomicI8>,
    /// A handle to the player's processing loop.
    pub handle: WeakSender<Command>,
    /// A handle to the player's block queue.
    pub block_handle: WeakSender<(Vector3<u16>, u8)>,
    /// The player's username.
    pub username: Weak<Mutex<String>>,
    /// The player's location.
    pub location: Weak<AtomicLocation>,
    /// Whether the player is connected. If this is false, the player should be dropped ASAP.
    pub connected: Weak<AtomicBool>,
    /// The player's UUID.
    pub uuid: Uuid
}

impl WeakPlayer {

    pub fn any_dropped(&self) -> bool {
        self.connected.upgrade().is_none() ||
            self.world.upgrade().is_none() ||
            self.id.upgrade().is_none() ||
            self.handle.upgrade().is_none() ||
            self.block_handle.upgrade().is_none() ||
            self.username.upgrade().is_none() ||
            self.location.upgrade().is_none()
    }
}

impl From<&Player> for WeakPlayer {
    fn from(value: &Player) -> Self {
        WeakPlayer {
            world: Arc::downgrade(&value.world),
            id: Arc::downgrade(&value.id),
            username: Arc::downgrade(&value.username),
            location: Arc::downgrade(&value.location),
            connected: Arc::downgrade(&value.connected),
            handle: value.handle.downgrade(),
            block_handle: value.block_handle.downgrade(),
            uuid: value.uuid
        }
    }
}

#[derive(Debug, Clone)]
pub enum Command {
    Disconnect {
        /// The reason for disconnection.
        reason: String
    },
    /// Initialize a player.
    Initialize {
        username: String
    },
    /// Sends the player to a world.
    SendTo { world: World },
    SetBlock { location: Vector3<u16>, id: u8 },
    SetLocation { location: Location },
    NotifyLeave { id: i8 },
    NotifyMove { id: i8, location: Location },
    NotifyJoin { id: i8, location: Location, name: String },
    Message { username: String, message: String },
}

impl Drop for Player {
    fn drop(&mut self) {
        if Arc::strong_count(&self.id) == 1
            && Arc::strong_count(&self.world) == 1
            && Arc::strong_count(&self.username) == 1
            && Arc::strong_count(&self.location) == 1
        {
            trace!("Running drop glue...");
            self.downgrade().notify_disconnect("Player struct dropped").block_on();
        }
    }
}

impl Player {
    /// Downgrades to a weak reference.
    pub fn downgrade(&self) -> WeakPlayer {
        self.into()
    }

    /// Create a new empty player.
    pub fn new(server: RunningServer, writer: OwnedWriteHalf) -> Player {
        let (tx, rx) = mpsc::channel(128);
        let (btx, brx) = mpsc::channel(256);

        let player = Player {
            world: Arc::new(Mutex::new(World::empty())),
            id: Arc::new(AtomicI8::new(-1)),
            handle: tx,
            block_handle: btx,
            username: Arc::new(Mutex::new(String::new())),
            location: Arc::new(Location {
                position: Vector3 { x: 0.into(), y: 0.into(), z: 0.into() },
                yaw: 0,
                pitch: 0,
            }.into()),
            connected: Arc::new(AtomicBool::new(true)),
            uuid: Uuid::new_v4()
        };

        tokio::spawn(player.downgrade().start_loops(rx, brx, server, writer));

        player
    }
}

macro_rules! g {
    ($e: expr; $k: stmt) => {{
        let Some(v) = {$e}.upgrade() else { $k };
        v
    }};
}

macro_rules! gb {
    ($e: expr) => {g!($e; break)};
}

impl WeakPlayer {

    /// Notifies the player that it has disconnected from the server.
    pub async fn notify_disconnect(self, reason: impl Into<String>) {
        if let Some(handle) = self.handle.upgrade() {
            // Pre-refactor, I was never handling this, so let's just make it easy
            let _ = handle.send(Command::Disconnect {
                reason: reason.into()
            }).await;
        }
    }

    /// Start the event loop for a player. This will handle all commands recieved over the Receiver, and start a task to periodically send heartbeats.
    #[allow(clippy::too_many_lines)]
    pub async fn start_loops(self, mut rx: mpsc::Receiver<Command>, brx: mpsc::Receiver<(Vector3<u16>, u8)>, server: RunningServer, writer: OwnedWriteHalf) {

        let (packet_timeout, ping_spacing) = {
            let config = server.config.lock();
            (config.packet_timeout, config.ping_spacing)
        };
        let default_world = server.default_world.clone();

        let (packet_send, packet_recv) = mpsc::channel(128);

        tokio::spawn(self.clone().start_packets(packet_recv, writer, packet_timeout));
        tokio::spawn(self.clone().start_block_queue(brx));
        tokio::spawn(self.clone().start_heartbeat(packet_send.clone(), ping_spacing, packet_timeout));

        'out: while let Some(command) = rx.recv().await {
            match command {
                Command::Disconnect { reason } => {
                    debug!("Disconnecting {}: {reason}", self.uuid);
                    let _ = packet_send.send(
                        Outgoing::Disconnect { reason }
                    ).await;
                    debug!("Sent disconnect packet");
                    let id = gb!(&self.id).load(Ordering::Relaxed);
                    {
                        let arc = gb!(self.world);
                        let lock = arc.lock();
                        lock.remove_player(id);
                    }
                    {
                        let mut conn_lock = server.connected_players.lock().await;
                        let arc = gb!(self.username);
                        let name_lock = arc.lock();
                        conn_lock.remove(&*name_lock);
                    }
                    gb!(self.connected).store(false, Ordering::Relaxed);
                    break;
                }
                Command::Initialize { username } => {
                    let (name, motd, operator): (String, String, bool);
                    #[allow(clippy::assigning_clones)] // Doesn't work with non-muts
                    let ban_reason = {
                        let lock = server.config.lock();
                        name = lock.name.clone();
                        motd = lock.motd.clone();
                        operator = lock.operators.contains(&username);
                        lock.banned_users.get(&username).map(|reason| format!("Banned: {reason}"))
                    };
                    if let Some(reason) = ban_reason {
                        let () = self.clone().notify_disconnect(reason).await;
                        continue;
                    }
                    let res = packet_send.send(Outgoing::ServerIdentification {
                        version: 7,
                        name,
                        motd,
                        operator,
                    }
                    ).await;
                    if let Err(e) = res {
                        let () = self.clone().notify_disconnect(format!("Connection failed: {e}")).await;
                    }

                    {
                        let arc = gb!(&self.username);
                        let mut lock = arc.lock();
                        lock.clone_from(&username);
                    }

                    {
                        let mut lock = server.connected_players.lock().await;
                        if lock.contains_key(&username) {
                            let () = self.clone().notify_disconnect("Player with same username already connected").await;
                            continue;
                        }
                        lock.insert(username, self.clone());
                    }

                    let _ = gb!(&self.handle).send(Command::SendTo {
                        world: default_world.clone()
                    }).await;
                }
                // TODO: Move this to a function on World
                Command::SendTo { world } => {
                    // The future checker doesn't consider dropping a mutex lock
                    // via std::mem::drop
                    // as making it unusable, so we do this instead.
                    let disconnect_reason = 'b: {

                        if world.is_full() {
                            break 'b "World is full".into();
                        }

                        // We hold the lock for the entire time here so that
                        // any block updates aren't pushed until the world data is done being sent
                        let data_lock = world.level_data.lock().await;

                        let dimensions = data_lock.dimensions;


                        // GZip level data
                        let data_slice = data_lock.raw_data.as_slice();

                        debug!("{} bytes to compress", data_slice.len());

                        if let Err(err) = packet_send.send(Outgoing::LevelInit).await {
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
                            if let Err(err) = packet_send.send(Outgoing::LevelDataChunk {
                                data_length: chunk.len() as u16,
                                data_chunk: Box::new(buf),
                                percent_complete: ((i as f32) / (chunk_count as f32) * 100.0) as u8
                            }).await {
                                break 'b format!("IO error: {err}")
                            }
                        }

                        if let Err(err) = packet_send.send(Outgoing::LevelFinalize {
                            size: dimensions
                        }).await {
                            break 'b format!("IO error: {err}")
                        }

                        drop(data_lock);

                        // Check again, preventing TOC-TOU bug
                        let Some(_) = world.add_player(self.clone()).await else {
                            debug!("World was full");
                            break 'b "World is full".into();
                        };

                        continue 'out;
                    };
                    let () = self.clone().notify_disconnect(disconnect_reason).await;
                }
                Command::SetBlock { location, id } => {
                    let _ = packet_send.send(
                        Outgoing::SetBlock {
                            position: location,
                            block: id
                        }
                    ).await;
                }
                Command::SetLocation { location } => {
                    gb!(&self.location).update(location);
                    let arc = gb!(&self.world);
                    let lock = arc.lock();
                    lock.move_player(gb!(&self.id).load(Ordering::Relaxed), location);
                }
                Command::NotifyLeave { id } => {
                    let _ = packet_send.send(
                        Outgoing::DespawnPlayer { id }
                    ).await;
                }
                Command::NotifyMove { id, location } => {
                    // TODO: Maybe should use update position?
                    let _ = packet_send.send(
                        Outgoing::TeleportPlayer { id, location }
                    ).await;
                }
                Command::NotifyJoin { mut id, location, name } => {
                    if id == gb!(&self.id).load(Ordering::Relaxed) {
                        id = -1;
                    }
                    let _ = packet_send.send(
                        Outgoing::SpawnPlayer {
                            id, location, name
                        }
                    ).await;
                }
                Command::Message { username, message } => {
                    let _ = packet_send.send(
                        Outgoing::Message {
                            id: -1,
                            // TODO: Is this the best way to do this?
                            message: format!("&f{username}: {message}")
                        }
                    ).await;
                }
            }
        }
    }

    /// Start the loop for sending packets to the client.
    async fn start_packets(self, mut recv: mpsc::Receiver<Outgoing>, mut writer: OwnedWriteHalf, timeout: Duration) {
        while let Some(packet) = recv.recv().await {
            trace!("Sending packet...");
            let Ok(()) = time::timeout(timeout, packet.store(&mut writer))
                .await
                .map_err(|_| io::Error::from(ErrorKind::TimedOut))
                .and_then(convert::identity) // Flatten error (.flatten() is not stable yet)
                else { break; };
            trace!("Sent packet {packet:?} to {}", self.uuid);
        }
    }

    /// Start the loop for placing blocks on the server.
    async fn start_block_queue(self, mut brx: mpsc::Receiver<(Vector3<u16>, u8)>) {
        'o: while let Some((location, id)) = brx.recv().await {
            if !gb!(&self.connected).load(Ordering::Relaxed) {
                break;
            }
            while {
                let arc = g!(&self.world; break 'o);
                let lock = arc.lock();
                !lock.set_block(location, id)
            } {
                // Wait a little before checking again
                time::sleep(Duration::from_millis(10)).await;
            }
        }
    }

    /// Start the heartbeat loop for a player.
    async fn start_heartbeat(self, send: Sender<Outgoing>, spacing: Duration, timeout: Duration) {
        let mut interval = time::interval(spacing);

        while self.connected.upgrade().is_some_and(|c| c.load(Ordering::Relaxed)) &&
            time::timeout(timeout, send.send(Outgoing::Ping))
            .await
            .map_err(|_| ())
            .map(|v| v.map_err(|_| ()))
            .and_then(convert::identity)
            .is_ok()
        {
            interval.tick().await;
        }

        // We timed out, shut off everything
        let () = self.notify_disconnect("Timed out").await;
    }

    /// Handle the packets for a player. This will block.
    pub async fn handle_packets(self, mut stream: OwnedReadHalf, server: RunningServer) {
        let verify = {
            let lock = server.config.lock();
            lock.kept_salts != 0
        };

        while self.connected.upgrade().is_some_and(|b| b.load(Ordering::Relaxed)) {

            // Using a match instead of .map_err since I need to break
            let res = match Incoming::load(&mut stream).await {
                Ok(v) => v,
                Err(err) => {
                    let () = self.notify_disconnect(format!("Connection died: {err}")).await;
                    break;
                }
            };

            trace!("Received packet {res:?} from {}", self.uuid);

            // Actually handle the packet
            match res {
                Incoming::PlayerIdentification { version, username, key } => {
                    if version != 0x07 {
                        let _ = gb!(self.handle).send(Command::Disconnect {
                            reason: format!("Failed to connect: Incorrect protocol version {version}")
                        }).await;
                        break;
                    }

                    let verified = !verify || {
                        let salts = server.last_salts.lock();

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
                        let _ = gb!(&self.handle).send(Command::Disconnect {
                            reason: "Failed to connect: Unauthorized".to_string()
                        }).await;
                        break;
                    }

                    let Ok(()) = gb!(&self.handle).send(Command::Initialize {
                        username
                    }).await else { break };
                }

                Incoming::SetBlock { position, state } => {
                    let Ok(()) = gb!(&self.block_handle).send((position, state)).await else { break };
                }

                Incoming::SetLocation { location } => {
                    let Ok(()) = gb!(&self.handle).send(
                        Command::SetLocation { location }
                    ).await else { break };
                }

                Incoming::Message { message } => {
                    let username = {
                        let arc = gb!(&self.username);
                        let lock = arc.lock();
                        lock.clone()
                    };

                    let res = {
                        server.commander.send(ServerCommand::SendMessage {
                            username,
                            message,
                        }).await
                    };

                    if res.is_err() {
                        let _ = gb!(self.handle).send(Command::Disconnect { reason: "Server loop died".into() }).await;
                        break;
                    }
                }
            }
        }
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
