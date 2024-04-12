use std::{io::ErrorKind, net::Ipv4Addr, sync::{Arc, Mutex}};
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
pub(crate) async fn start(server: Arc<Mutex<Server>>) -> Result<(), io::Error> {
    let config = {
        let lock = server.lock().expect("other thread panicked");
        lock.config.clone()
    };

    let listener = TcpListener::bind((
        Ipv4Addr::new(127, 0, 0, 1),
        config.port
    )).await?;

    let hb_config = config.clone();
    let hb_server = server.clone();
    let heartbeat_task: JoinHandle<Result<(), io::Error>> = tokio::spawn(async move {
        let config = hb_config;
        let server = hb_server;
        let mut fails = 0;
        let mut heartbeat_rand = StdRng::from_entropy();
        let client = reqwest::Client::new();
        loop {
            let next_wakeup = Instant::now() + config.heartbeat_spacing;
            // Generate a new salt
            let new_salt = heartbeat_rand.salt();
            let req = {
                let lock = server.lock().expect("other thread panicked");
                client.post(&config.heartbeat_url)
                    .query(&[
                        ("port", config.port),

                    ]).build().map_err(|err| io::Error::new(
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

#[derive(Debug, Clone, PartialEq, Hash)]
pub struct PlayerState {
    /// The player's username.
    pub username: String
}

/// Handle a single connection to the server. This is long-running!
async fn handle_stream(config: Config, server: Arc<Mutex<Server>>, stream: TcpStream) {
    let (tx, mut rx) = mpsc::channel::<Outgoing>(100);
    let htx = tx.clone();
    let (mut read, mut write) = stream.into_split();
    
    let player_state = Arc::new(Mutex::new(PlayerState {
        username: String::new()
    }));

    let recv_server = server.clone();
    let send_player = player_state.clone();

    let recv_task = tokio::spawn(async move {
        while let Some(packet) = rx.recv().await {
            let res = time::timeout(config.packet_timeout, packet.store(&mut write)).await;
            match res {
                Err(_) => { // Timeout
                    let _ = time::timeout(
                        config.packet_timeout,
                        Outgoing::Disconnect {
                            reason: format!("Connection timed out")
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
                let mut lock = recv_server.lock().expect("other thread panicked");
                let state = player_state.lock().expect("other thread panicked");
                lock.disconnect(&state.username);
            }
        }
    });

    let send_task: JoinHandle<()> = tokio::spawn(async move {
        let player = send_player;
        while !tx.is_closed() {
            match {
                let res = Incoming::load(&mut read).await;
                match res {
                    Ok(packet) => packet,
                    Err(e) => {
                        let _ = tx.send(Outgoing::Disconnect {
                            reason: format!("Connection error: {e}")
                        }).await;

                        break;
                    }
                }
            } {
                Incoming::PlayerIdentification { version, username, key } => {
                    if version != 0x07 {
                        let _ = tx.send(Outgoing::Disconnect {
                            reason: format!("Failed to connect: incorrect version 0x{version:02x}")
                        }).await;
                        break;
                    }
                    let server = server.lock().expect("other thread panicked");

                    if server.last_salts.is_empty() || {
                        let mut res = false;
                        for salt in server.last_salts.iter() {
                            let server_key = md5::compute(salt.to_owned() + &username);
                            if &*server_key == key.as_bytes() {
                                res = true;
                                break;
                            }
                        }
                        res
                    } {

                    }
                }
                Incoming::SetBlock { position, state } => {
                    /// Get the world that the player is in
                    let res = {
                        let player_name = &player.lock().expect("other thread panicked").username;
                        server.lock().expect("other thread panicked").players_connected.get(player_name)
                    };
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
