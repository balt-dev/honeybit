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
use std::sync::OnceLock;
use codepage_437::{FromCp437, CP437_WINGDINGS, ToCp437, Cp437Error};
use itertools::Itertools;
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
    pub username: Arc<OnceLock<String>>,
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
    pub username: Weak<OnceLock<String>>,
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
        /// Sends the player to a world.
        pub async fn send_to(&self, world: World) => SendTo;
        /// Sets a block for the player.
        pub async fn set_block(&self, id: u8, position: Vector3<u16>) => SetBlock;
        /// Notifies the player that another player has moved.
        pub async fn notify_move(&self, id: i8, location: Location) => NotifyMove;
        /// Notifies the player of the server's supported protocol extensions.
        pub async fn send_ext_info(&self) => NotifyExtensions;
        /// Sets the player's operator status.
        pub async fn notify_operator(&self, operator: bool) => SetOperator;
    }
    
    /// Notifies the player that it has disconnected.
    #[inline]
    pub async fn notify_disconnect(&self, reason: impl Into<String>) {        
        if let Some(handle) = self.handle.upgrade() {
            let _ = handle.send(Command::Disconnect{ reason: reason.into() }).await;
        }
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
    SetOperator { operator: bool }
}

impl Drop for Player {
    fn drop(&mut self) {
        // Empty, still makes sure it runs so players aren't dropped until the scope ends
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
            username: Arc::default(),
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
                        let name = gb!(self.username).get().cloned();
                        if let Some(ref name) = name {
                            conn_lock.remove(name);
                        }
                        name
                    };

                    if let Some(username) = username {
                        let message = format!("&4[&c-&4] &f{username}");
                        tokio::spawn(async move { server.send_message(message).await; });
                    }

                    gb!(self.connected).store(false, Ordering::Relaxed);
                    break;
                }
                Command::Initialize { username } => {
                    
                    let (name, motd, operator, max_players): (String, String, bool, usize);
                    #[allow(clippy::assigning_clones)] // Doesn't work with non-mutable variables
                    let ban_reason = {
                        let lock = server.config.lock();
                        name = lock.name.clone();
                        motd = lock.motd.clone();
                        operator = lock.operators.contains(&username);
                        max_players = lock.max_players;
                        lock.banned_users.get(&username).map(|reason| format!("Banned: {reason}"))
                    };
                    
                    server.collect_garbage();
                    let player_count = server.connected_players.lock().await.len();

                    let res = packet_send.send(Outgoing::ServerIdentification {
                            version: 7,
                            name,
                            motd,
                            operator,
                        }
                    ).await;

                    if player_count >= max_players {
                        self.notify_disconnect("Server is full").await;
                        continue;
                    }
                    
                    if let Some(reason) = ban_reason {
                        self.notify_disconnect(reason).await;
                        continue;
                    }

                    if let Err(e) = res {
                        self.notify_disconnect(format!("Connection failed: {e}")).await;
                        continue;
                    }

                    if username.find(|c: char| c.is_ascii_whitespace()).is_some() {
                        self.notify_disconnect("Username cannot have whitespace".to_string()).await;
                        continue;
                    }

                    gb!(&self.username).get_or_init(|| username.clone());

                    let message = format!("&2[&a+&2] &f{username}");

                    {
                        let mut lock = server.connected_players.lock().await;
                        if lock.contains_key(&username) {
                            self.notify_disconnect("Player with same username already connected").await;
                            continue;
                        }
                        lock.insert(username, self.clone());
                    }

                    server.send_message(message).await;

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
                Command::SetOperator { operator } => {
                    let _ = packet_send.send(
                        Outgoing::UpdateUser { operator }
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

                Incoming::Message { message, mut append } => {
                    let max_length = {
                        let lock = server.config.lock();
                        lock.max_message_length
                    };

                    let cp437_msg = String::from_cp437(Vec::from(message), &CP437_WINGDINGS);
                    let append_message = if append { &cp437_msg } else { cp437_msg.trim_end() };
                    message_buffer.push_str(append_message);

                    if message_buffer.len() > max_length {
                        message_buffer.truncate(max_length);
                        append = false;
                    }

                    if !append {
                        let operator = {
                            let conf = server.config.lock();
                            let username_arc = self.username.upgrade();
                            username_arc.is_some_and(
                                |name| name.get().is_some_and(
                                    |name| conf.operators.contains(name)
                                )
                            )
                        };

                        // Process commands
                        if message_buffer.starts_with('/') {
                            match self.execute_command(message_buffer, server.clone()).await {
                                Ok(b) => if b { break },
                                Err(error_message) => {
                                    self.send_message(format!("&4[&c!&4] &f{error_message}")).await;
                                }
                            }
                           message_buffer = String::new();
                           continue;
                        }

                        let username = gb!(&self.username).get().cloned().unwrap_or_default();
                        let message = if operator {
                            format!("&3[&b{username}&3] &f{message_buffer}")
                        } else {
                            format!("&8[&7{username}&8] &f{message_buffer}")
                        };
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

    /// Executes a single command.
    #[allow(clippy::too_many_lines)]
    async fn execute_command(&self, raw_message: String, server: RunningServer) -> Result<bool, String> {

        let operator = {
            let conf = server.config.lock();
            let username_arc = self.username.upgrade()
                .ok_or("Username dropped".to_string())?;
            let Some(username) = username_arc.get()
                else { return Err("Username not initialized".to_string())};
            conf.operators.contains(username)
        };

        let mut arguments = raw_message[1..].split_ascii_whitespace();
        let name = arguments.next().unwrap_or("");
        match name {
            "world" => match arguments.next() {
                Some("join") => {
                    // FIXME: This requires nightly.
                    let Some(world_name) = arguments.remainder() else {
                        return Err("No world name specified".into())
                    };
                    let lock = server.worlds.lock().await;
                    let Some(world) = lock.get(world_name).cloned() else {
                        return Err(format!("World \"{world_name}\" doesn't exist"))
                    };
                    self.send_to(world).await;
                },
                Some("list") => {
                    self.send_message("&6[&eWorld List&6]").await;
                    let worlds = {
                        let lock = server.worlds.lock().await;
                        lock.keys().cloned().collect_vec()
                    };
                    for world in worlds {
                        self.send_message(format!("- {world}")).await;
                    }
                },
                Some("save") if operator => {
                    let Some(world) = self.world.upgrade() else { return Ok(false) };
                    let data = world.lock().data.clone();
                    let name = data.lock().await.name.clone();
                    server.send_message(format!("&6[&e@&6] &fSaving world \"{name}\"...")).await;
                    let world = world.lock().clone();
                    if let Err(err) = world.save().await {
                        server.send_message("&4[&c!&4] Failed to save! See logs for details.").await;
                        warn!("Failed to save world \"{name}\": {err}");
                        return Ok(false);
                    };
                    info!("Saved world \"{name}\"");
                    server.send_message("&6[&e@&6] &fWorld saved!".to_string()).await;
                },
                Some("create") if operator => {
                    let Some(length) = arguments.next() else { return Err("No length specified".into()) };
                    let Some(width) = arguments.next() else { return Err("No width specified".into()) };
                    let Some(height) = arguments.next() else { return Err("No height specified".into()) };
                    let Some(generator) = arguments.next() else { return Err("No generator specified, see /world generators".into()) };
                    let seed: u64 = match arguments.next() {
                        Some(seed) => seed.parse().unwrap_or_else(
                            |_| fxhash::hash64(seed.as_bytes())
                        ),
                        None => rand::random()
                    };
                    
                    let length: u16 = length.parse().map_err(|err| format!("Invalid length: {err}"))?;
                    let width: u16 = width.parse().map_err(|err| format!("Invalid width: {err}"))?;
                    let height: u16 = height.parse().map_err(|err| format!("Invalid height: {err}"))?;
                    let dimensions = Vector3 { x: length, z: width, y: height };

                    let lock = server.generators.lock();
                    let Some(generator) = lock.get(generator) else { return Err("Invalid generator {generator}".into())};

                }
                Some("generators") if operator => {
                    self.send_message("&6[&eWorld Generators&6]").await;
                    let keys: Vec<_> = {
                        let lock = server.generators.lock();
                        lock.keys().cloned().collect()
                    };
                    for gen in keys {
                        self.send_message(format!("- {gen}")).await;
                    }
                }
                Some(cmd) => return Err(format!("Invalid subcommand {cmd}")),
                None => {
                    self.send_message("/world").await;
                    self.send_message("- join &b<name>").await;
                    self.send_message("- list").await;
                    if operator {
                        self.send_message("&e- save").await;
                    }
                }
            },
            "stop" if operator => {
                server.stop().await;
                return Ok(true);
            },
            "w" => {
                let Some(name) = arguments.next() else {
                    return Err("No username specified".into())
                };
                let players = server.connected_players.lock().await;
                let Some(player) = players.get(name).cloned() else {
                    return Err(format!("User {name} is not online"))
                };
                let Some(own_name) = self.username.upgrade().and_then(|v| v.get().cloned()) else {
                    return Err(String::new()) // They won't see this anyways
                };
                let Some(message) = arguments.remainder() else {
                    return Err("Message must be non-empty".into())
                };
                self.send_message(format!("&0[&8{own_name} -> {name}&0] &7{message}")).await;
                player.send_message(format!("&0[&{own_name} -> {name}&0] &7{message}")).await;
            }
            "locate" => {
                let Some(name) = arguments.next() else {
                    return Err("No username specified".into())
                };
                let players = server.connected_players.lock().await;
                let Some(world) = players.get(name).cloned().and_then(|player| player.world.upgrade()) else {
                    return Err(format!("User {name} is not online"))
                };
                let world = world.lock().clone();
                self.send_message(format!("{name} is in \"{}\"", world.data.lock().await.name)).await;
            }
            "players" => {
                let players = server.connected_players.lock().await;
                self.send_message("&3[&bPlayer List&3]").await;
                for name in players.keys() {
                    self.send_message(name.to_string()).await;
                }
            }
            "op" if operator => {
                let Some(name) = arguments.next() else {
                    return Err("No username specified".into())
                };
                let players = server.connected_players.lock().await;

                {
                    let mut conf = server.config.lock();
                    conf.operators.insert(name.to_string());
                }

                if let Some(player) = players.get(name) {
                    player.send_message("&3[&b#&3] &fGranted operator permissions".to_string()).await;
                    player.notify_operator(true).await;
                };

                self.send_message(format!("&3[&b#&3] &fGranted operator permissions to {name}")).await;
            },
            "deop" if operator => {
                let Some(name) = arguments.next() else {
                    return Err("No username specified".into())
                };
                let players = server.connected_players.lock().await;

                {
                    let mut conf = server.config.lock();
                    conf.operators.remove(name);
                }

                if let Some(player) = players.get(name) {
                    player.send_message("&3[&b#&3] &fOperator permissions revoked".to_string()).await;
                    player.notify_operator(false).await;
                };

                self.send_message(format!("&3[&b#&3] &fRevoked operator permissions from {name}")).await;
            }
            "kick" if operator => {
                let Some(name) = arguments.next() else {
                    return Err("No username specified".into())
                };
                let players = server.connected_players.lock().await;
                let Some(player) = players.get(name) else {
                    return Err("Player is offline".into())
                };
                player.notify_disconnect(
                    format!("Kicked: {}", arguments.remainder().unwrap_or("No reason given"))
                ).await;

                self.send_message(format!("&3[&b#&3] &fKicked {name}")).await;
            },
            "ban" if operator => {
                let Some(name) = arguments.next() else {
                    return Err("No username specified".into())
                };
                let players = server.connected_players.lock().await;
                let reason = arguments.remainder().unwrap_or("No reason given");
                if let Some(player) = players.get(name) {
                    player.notify_disconnect(
                        format!("Banned: {reason}")
                    ).await;
                };

                self.send_message(format!("&3[&b#&3] &fBanned {name}")).await;

                let mut config = server.config.lock();
                config.banned_users.insert(name.to_string(), reason.to_string());
            },
            "unban" if operator => {
                let Some(name) = arguments.next() else {
                    return Err("No username specified".into())
                };

                self.send_message(format!("&3[&b#&3] &fUnbanned {name}")).await;

                let mut config = server.config.lock();
                config.banned_users.remove(name);
            },
            "help" => {
                self.send_message("&5[&dCommand List&5]").await;
                self.send_message("- /world").await;
                self.send_message("  - /world join <name>").await;
                self.send_message("  - /world list").await;
                if operator {
                    self.send_message("&b  - /world save [name]").await;
                    self.send_message("&b  - /world generators").await;
                    self.send_message("&b  - /world create <length> <width> <height> <generator> [seed]").await
                }
                self.send_message("- /w <user> <message>").await;
                self.send_message("- /locate <user>").await;
                self.send_message("- /players").await;
                if operator {
                    self.send_message("&b- /op <name>").await;
                    self.send_message("&b- /deop <name>").await;
                    self.send_message("&b- /kick <name> [reason]").await;
                    self.send_message("&b- /ban <name> [reason]").await;
                    self.send_message("&b- /unban <name>").await;
                    self.send_message("&b- /stop").await;
                }
            }
            _ => return Err(format!("Invalid command {name}"))
        }
        Ok(false)
    }
}
