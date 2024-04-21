use std::{
    convert,
    io::{self, ErrorKind},
    sync::{
        Arc,
        atomic::AtomicI8,
        atomic::Ordering,
    },
    time::Duration,
    sync::atomic::AtomicBool,
    sync::Weak
};
use std::collections::BTreeMap;
use std::sync::OnceLock;
use codepage_437::{FromCp437, CP437_WINGDINGS, ToCp437, Cp437Error};
use dynfmt::{Format, SimpleCurlyFormat};
use pollster::FutureExt as _;
use crate::{
    packets::{
        IncomingPacketType as _,
        OutgoingPacketType as _,
        Incoming,
        Outgoing,
        Vector3,
        Location
    },
    packets::AtomicLocation,
    server::RunningServer,
    world::World
};
use tokio::{
    net::tcp::{OwnedReadHalf, OwnedWriteHalf},
    sync::mpsc::{self, Receiver, Sender, WeakSender},
    time,
};
use uuid::Uuid;
use parking_lot::Mutex;
use crate::packets::SupportedExtensions;

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
    pub uuid: Uuid,
    /// The protocol extensions that the player supports.
    pub supported_exts: Arc<OnceLock<SupportedExtensions>>
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
    pub uuid: Uuid,
    /// The protocol extensions the player supports.
    pub supported_exts: Weak<OnceLock<SupportedExtensions>>
}

macro_rules! command_wrapper {
    ($(
        $(#[$outer:meta])*
        pub async fn $name: ident(&self $(,$($var: ident: $ty: ty),*)?) => $command: ident;
    )+) => {$(
        $(#[$outer])*
        #[inline]
        pub async fn $name(&self$(, $($var: impl Into<$ty>),+)?) {
            if let Some(handle) = self.handle.upgrade() {
                let _ = handle.send(Command::$command$( { $($var: $var.into()),+ })?).await;
            }
        }
    )+};
}

/// Implementations to better handle cross-thread communication
impl WeakPlayer {
    command_wrapper! {
        /// Sets the player's location.
        pub async fn set_location(&self, location: Location) => SetLocation;
        /// Notifies the player that another player has joined the world that they're in.
        pub async fn notify_join(&self, id: i8, location: Location, name: String) => NotifyJoin;
        /// Notifies the player that another player has left the world that they're in.
        pub async fn notify_left(&self, id: i8) => NotifyLeave;
        /// Sends the player a message in chat.
        pub async fn send_message(&self, message: String) => Message;
        /// Notifies the player that it has disconnected.
        pub async fn notify_disconnect(&self, reason: String) => Disconnect;
        /// Sends the player to a world.
        pub async fn send_to(&self, world: World) => SendTo;
        /// Sets a block for the player.
        pub async fn set_block(&self, id: u8, position: Vector3<u16>) => SetBlock;
        /// Notifies the player that another player has moved.
        pub async fn notify_move(&self, id: i8, location: Location) => NotifyMove;
        /// Notifies the player of the server's supported protocol extensions.
        pub async fn send_ext_info(&self) => NotifyExtensions;
    }

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
            uuid: value.uuid,
            supported_exts: Arc::downgrade(&value.supported_exts)
        }
    }
}

/// Player "commands". These are used for cross-thread communication.
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
    /// Sets a client-side block for the player.
    SetBlock { position: Vector3<u16>, id: u8 },
    SetLocation { location: Location },
    NotifyLeave { id: i8 },
    NotifyMove { id: i8, location: Location },
    NotifyJoin { id: i8, location: Location, name: String },
    Message { message: String },
    NotifyExtensions,
}

impl Drop for Player {
    fn drop(&mut self) {
        if Arc::strong_count(&self.id) == 1
            && Arc::strong_count(&self.world) == 1
            && Arc::strong_count(&self.username) == 1
            && Arc::strong_count(&self.location) == 1
        {
            trace!("Player {} dropped", self.uuid);
            self.downgrade().notify_disconnect("Player dropped (if you're seeing this, a thread likely panicked)").block_on();
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
            world: Arc::default(),
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
            uuid: Uuid::new_v4(),
            supported_exts: Arc::default()
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
    /// Start the event loop for a player. This will handle all commands received over the Receiver, and start a task to periodically send heartbeats.
    #[allow(clippy::too_many_lines)]
    pub async fn start_loops(self, mut rx: Receiver<Command>, brx: Receiver<(Vector3<u16>, u8)>, server: RunningServer, writer: OwnedWriteHalf) {

        let (packet_timeout, ping_spacing) = {
            let config = server.config.lock();
            (config.packet_timeout, config.ping_spacing)
        };
        let default_world = server.default_world.clone();

        let (packet_send, packet_recv) = mpsc::channel(128);

        tokio::spawn(self.clone().start_packets(packet_recv, writer, packet_timeout));
        tokio::spawn(self.clone().start_block_queue(brx));
        tokio::spawn(self.clone().start_heartbeat(packet_send.clone(), ping_spacing, packet_timeout));

        while let Some(command) = rx.recv().await {
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
                    let username = {
                        let mut conn_lock = server.connected_players.lock().await;
                        let arc = gb!(self.username);
                        let name_lock = arc.lock();
                        conn_lock.remove(&*name_lock);
                        name_lock.clone()
                    };
                    
                    let message = {
                        let conf = server.config.lock();
                        SimpleCurlyFormat.format(&conf.leave_format, BTreeMap::from([
                            ("username", &username)
                        ]))
                            .map(|v| v.to_string())
                            .map_err(|e| format!("{e}")) // Do this to remove the borrow in the error
                    }.unwrap_or_else(|e| {
                        warn!("Failed to format leave message: {e}");
                        warn!("Falling back to default...");
                        format!("&4[&c-&4] &f{username}")
                    });

                    tokio::spawn(async move {server.send_message(message).await;});
                    
                    gb!(self.connected).store(false, Ordering::Relaxed);
                    break;
                }
                Command::Initialize { username } => {
                    
                    let (name, motd, operator): (String, String, bool);
                    #[allow(clippy::assigning_clones)] // Doesn't work with non-mutable variables
                    let ban_reason = {
                        let lock = server.config.lock();
                        name = lock.name.clone();
                        motd = lock.motd.clone();
                        operator = lock.operators.contains(&username);
                        lock.banned_users.get(&username).map(|reason| format!("Banned: {reason}"))
                    };
                    
                    if let Some(reason) = ban_reason {
                        debug!("Banned player {username} attempted to join");
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

                    let message = {
                        let conf = server.config.lock();
                        SimpleCurlyFormat.format(&conf.join_format, BTreeMap::from([
                            ("username", &username)
                        ]))
                            .map(|v| v.to_string())
                            .map_err(|e| format!("{e}")) // Do this to remove the borrow in the error
                    }.unwrap_or_else(|e| {
                        warn!("Failed to format join message: {e}");
                        warn!("Falling back to default...");
                        format!("&2[&a+&2] &f{username}")
                    });
                    
                    server.send_message(message).await;

                    {
                        let mut lock = server.connected_players.lock().await;
                        if lock.contains_key(&username) {
                            let () = self.clone().notify_disconnect("Player with same username already connected").await;
                            continue;
                        }
                        lock.insert(username, self.clone());
                    }

                    self.send_to(default_world.clone()).await;
                }
                Command::SendTo { world: dst_world } => {
                    let Some(src_world) = self.world.upgrade() else { continue };
                    let Some(id) = self.id.upgrade() else { continue };
                    {
                        let lock = src_world.lock();
                        lock.remove_player(id.load(Ordering::Relaxed));
                    }
                    dst_world.add_player(self.clone(), packet_send.clone()).await;
                }
                Command::SetBlock { position: location, id } => {
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
                Command::Message { mut message } => {
                    let supported_exts = gb!(&self.supported_exts)
                        .get()
                        .copied()
                        .unwrap_or(SupportedExtensions::empty());
                        
                    loop {
                        let Some(idx) = (
                            if supported_exts.contains(SupportedExtensions::FULL_CP437) {
                                let Err(Cp437Error { representable_up_to: idx}) =
                                    message.to_cp437(&CP437_WINGDINGS)
                                    else {break};
                                Some(idx)
                            } else if supported_exts.contains(SupportedExtensions::EMOTE_FIX) {
                                message.find(|c: char| !c.is_ascii())
                            } else {
                                message.find(|c: char| !c.is_ascii() && c.is_control())
                            }
                        ) else {
                            break
                        };
                        let chr = message[idx..].chars().next().expect("should not fail except in the middle of the string");
                        message = String::new() + &message[..idx] + "?" + &message[idx + chr.len_utf8()..];
                    }
                    let Ok(encoded) = message.to_cp437(&CP437_WINGDINGS) else {unreachable!()};
                    let mut buf;
                    let mut iter = encoded.chunks(64).peekable();
                    // Do desugared loop as to not move the iterator
                    loop {
                        let Some(chunk) = iter.next() else { break };
                        buf = [b' '; 64];
                        buf[..chunk.len()].copy_from_slice(chunk);
                        let longer_supported = supported_exts.contains(SupportedExtensions::LONGER_MESSAGES);
                        if packet_send.send(
                            Outgoing::Message {
                                id: i8::from(longer_supported && iter.peek().is_some()),
                                message: buf
                            }
                        ).await.is_err() { break }
                    }
                }
                Command::NotifyExtensions => {
                    let _ = packet_send.send(
                        Outgoing::ExtInfoEntry
                    ).await;
                }
            }
        }
    }

    /// Start the loop for sending packets to the client.
    async fn start_packets(self, mut recv: Receiver<Outgoing>, mut writer: OwnedWriteHalf, timeout: Duration) {
        while let Some(packet) = recv.recv().await {
            let Ok(()) = time::timeout(timeout, packet.store(&mut writer))
                .await
                .map_err(|_| io::Error::from(ErrorKind::TimedOut))
                .and_then(convert::identity) // Flatten error (.flatten() is not stable yet)
            else { break };
            if !matches!(packet, Outgoing::TeleportPlayer { .. }) {
                trace!("Sent packet {packet:?} to {}", self.uuid);
            }
        }
    }

    /// Start the loop for placing blocks on the server.
    async fn start_block_queue(self, mut brx: Receiver<(Vector3<u16>, u8)>) {
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
    #[allow(clippy::too_many_lines)]
    pub async fn handle_packets(self, mut stream: OwnedReadHalf, server: RunningServer) {
        let verify = {
            let lock = server.config.lock();
            lock.kept_salts != 0
        };

        let mut message_buffer = String::new();

        while self.connected.upgrade().is_some_and(|b| b.load(Ordering::Relaxed)) {

            // Using a match instead of .map_err since I need to break
            let res = match Incoming::load(&mut stream).await {
                Ok(v) => v,
                Err(err) => {
                    let () = self.notify_disconnect(format!("Connection died: {err}")).await;
                    break;
                }
            };
            
            if !matches!(res, Incoming::SetLocation { .. }) {
                trace!("Received packet {res:?} from {}", self.uuid);
            }
            
            // Actually handle the packet
            match res {
                Incoming::PlayerIdentification { version, username, key, cpe_supported } => {
                    if version != 0x07 {
                        self.notify_disconnect(format!("Failed to connect: Incorrect protocol version {version}")).await;
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
                        self.notify_disconnect("Failed to connect: Unauthorized").await;
                        break;
                    }
                    
                    if cpe_supported {
                        self.send_ext_info().await;
                        let supported_exts = match Incoming::load(&mut stream).await {
                            Ok(Incoming::ExtInfoEntry {supported_exts}) => supported_exts,
                            Ok(_) => {
                                let () = self.notify_disconnect("Client replied inappropriately to ExtInfo packet".to_string()).await;
                                break;
                            }
                            Err(err) => {
                                let () = self.notify_disconnect(format!("Connection died: {err}")).await;
                                break;
                            }
                        };
                        
                        let Some(once) = self.supported_exts.upgrade() else { break };
                        once.get_or_init(|| supported_exts);
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

                Incoming::Message { message, append } => {
                    let max_length = {
                        let lock = server.config.lock();
                        lock.max_message_length
                    };

                    let cp437_msg = String::from_cp437(Vec::from(message), &CP437_WINGDINGS);
                    let append_message = if append { &cp437_msg } else { cp437_msg.trim_end() };
                    if message_buffer.len() + append_message.len() > max_length {
                        self.notify_disconnect(format!("Sent message longer than configured maximum ({max_length})")).await;
                        break;
                    }
                    message_buffer.push_str(append_message);

                    if !append {
                        let username = {
                            let arc = gb!(&self.username);
                            let lock = arc.lock();
                            lock.clone()
                        };
                        let message = {
                            let conf = server.config.lock();
                            SimpleCurlyFormat.format(&conf.message_format, BTreeMap::from([
                                ("username", &username), ("message", &message_buffer)
                            ]))
                                .map(|v| v.to_string())
                                .map_err(|e| format!("{e}")) // Do this to remove the borrow in the error
                        }.unwrap_or_else(|e| {
                            warn!("Failed to format chat message: {e}");
                            warn!("Falling back to default...");
                            format!("&8[&7{username}&8] &f{message_buffer}")
                        });
                        server.send_message(message).await;
                        message_buffer = String::new();
                    }
                },
                Incoming::ExtInfoEntry { .. } => {
                    self.notify_disconnect("Got ExtInfo at unexpected time").await;
                    break;
                }
            }
        }
    }
}
