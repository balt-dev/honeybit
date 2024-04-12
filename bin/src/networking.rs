//! Handles general networking, just gluing to the lib

use std::{io::ErrorKind, net::Ipv4Addr, sync::{Arc, Mutex, RwLock, OnceLock}};
use rand::{rngs::StdRng, SeedableRng};
use tokio::{
    io,
    net::{TcpListener, TcpStream},
    time,
    sync::mpsc,
    join,
    task::JoinHandle
};
use tokio::time::Instant;
use oxine::{
    networking::{IncomingPacketType, OutgoingPacketType},
    packets::{Incoming, Outgoing},
    server::{Config, SaltExt, Server}
};

/// Starts the networking section of the server.
pub(crate) async fn start(server: Arc<RwLock<Server>>) -> Result<(), io::Error> {
    let config = {
        let lock = server.read().expect("other thread panicked");
        lock.config.clone()
    };

    let listener = TcpListener::bind((
        Ipv4Addr::new(127, 0, 0, 1),
        config.port
    )).await?;

    let hb_config = config.clone();
    let hb_server = server.clone();

    // This won't get dropped until the loop below ends
    let _heartbeat_task: JoinHandle<Result<(), io::Error>> = tokio::spawn(async move {
        let config = hb_config;
        let server = hb_server;
        let mut fails = 0;
        let mut heartbeat_rand = StdRng::from_entropy();
        let client = reqwest::Client::new();
        loop {
            let next_wakeup = Instant::now() + config.heartbeat_spacing;
            // Generate a new salt
            let req = {
                let (salt, user_count);
                {
                    let mut lock = server.write().expect("other thread panicked");
                    user_count = lock.players_connected.len();
                    salt = if config.kept_salts == 0 {
                        String::new()
                    } else {
                        let new_salt = heartbeat_rand.salt();
                        if lock.last_salts.len() < config.kept_salts {
                            lock.last_salts.push_front(new_salt.clone());
                            new_salt
                        } else {
                            // Shift the old ones to the back and insert the new one, without allocating
                            let back = lock.last_salts.back_mut().expect("salt list is not empty here");
                            let _ = std::mem::replace(back, new_salt.clone());
                            lock.last_salts.rotate_right(1);
                            new_salt
                        }
                    };
                }
                client.post(&config.heartbeat_url)
                    .query(&[("port", config.port)])
                    .query(&[("max", config.max_players)])
                    .query(&[("name", &config.name)])
                    .query(&[("public", &config.public)])
                    .query(&[("version", 7)])
                    .query(&[("salt", salt)])
                    .query(&[("users", user_count)])
                    .build().map_err(|err| io::Error::new(
                        ErrorKind::Other,
                        err
                    ))
            }?;

            while time::timeout(config.packet_timeout, 
                client.execute(req.try_clone().expect("streams aren't being used here"))
            ).await.is_err() {
                // Failed to ping heartbeat URL.
                if fails >= config.heartbeat_retries {
                    return Err(io::Error::new(ErrorKind::TimedOut, "Failed to connect to heartbeat URL."))
                }
                fails += 1;
            }
            fails = 0;
            
            time::sleep_until(next_wakeup).await;
        }
    });

    loop {
        let connection = listener.accept().await;

        let Ok((stream, _)) = connection else {
            let err = connection.unwrap_err();
            error!("TCP connection failed.");
            error!("{err}");
            continue;
        };

        tokio::spawn(handle_stream(config.clone(), server.clone(), stream));
    }
}

/// Handle a single connection to the server. This is long-running!
async fn handle_stream(config: Config, server: Arc<RwLock<Server>>, stream: TcpStream) {
    let (tx, mut rx) = mpsc::channel::<Outgoing>(100);
    let htx = tx.clone();
    let (mut read, mut write) = stream.into_split();
    
    let player_name = Arc::new(OnceLock::new());

    let recv_server = server.clone();
    let recv_name = player_name.clone();

    let recv_task = tokio::spawn(async move {
        while let Some(packet) = rx.recv().await {
            let res = time::timeout(config.packet_timeout, packet.store(&mut write)).await;
            match res {
                Err(_) => { // Timeout
                    let _ = time::timeout(
                        config.packet_timeout,
                        Outgoing::Disconnect {
                            reason: "Connection timed out".to_string()
                        }.store(&mut write)
                    ).await;
                    rx.close();
                    break;
                },
                Ok(Err(err)) => { // Connection error
                    let _ = time::timeout(
                        config.packet_timeout,
                        Outgoing::Disconnect {
                            reason: format!("Connection error: {err}")
                        }.store(&mut write)
                    ).await;
                    rx.close();
                    break;
                },
                Ok(Ok(_)) => {}
            }
            if let Outgoing::Disconnect { .. } = packet {
                rx.close();
            }
            if rx.is_closed() {
                if let Some(username) = recv_name.get() {
                    let mut lock = recv_server.write().expect("other thread panicked");
                    lock.disconnect(username);
                }
            }
        }
    });

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
                    let Some((world, id)) = player_name.get().and_then(|name| lock.players_connected.get(name).cloned()) 
                        else { continue /* We aren't ready to recieve these yet */ };
                    
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
