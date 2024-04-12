use std::{io::ErrorKind, net::Ipv4Addr, sync::{Arc, Mutex}};
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
    
    let mut heartbeat_rand = rand::thread_rng();

    let heartbeat_task: JoinHandle<Result<(), io::Error>> = tokio::spawn(async move {
        let mut fails = 0;
        let client = reqwest::Client::new();
        loop {
            let next_wakeup = Instant::now() + config.heartbeat_spacing;
            // Generate a new salt
            let new_salt = heartbeat_rand.salt();

            let req = client.post(config.heartbeat_url)
                .query(&[

                ]).build().map_err(|err| io::Error::new(
                    ErrorKind::Other,
                    err
                ))?;

            if time::timeout(config.packet_timeout, client.execute(req)).await.is_err() {
                // Failed to ping heartbeat URL.
                if fails >= config.heartbeat_retries {
                    return Err(io::Error::new(ErrorKind::TimedOut, "Failed to connect to heartbeat URL."))
                }

            }
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
    /// The world the player is in.
    pub current_world: String,
    /// The ID of the player.
    pub id: i8
}

/// Handle a single connection to the server
async fn handle_stream(config: Config, server: Arc<Mutex<Server>>, stream: TcpStream) {
    let (tx, mut rx) = mpsc::channel::<Outgoing>(100);
    let htx = tx.clone();
    let (mut read, mut write) = stream.into_split();
    
    let player_state = Arc::new(Mutex::new(PlayerState {
        current_world: config.default_world.clone(),
        id: -1
    }));

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
                },
                Ok(Err(err)) => { // Connection error
                    let _ = time::timeout(
                        config.packet_timeout,
                        Outgoing::Disconnect {
                            reason: format!("Connection error: {err}")
                        }.store(&mut write)
                    ).await;
                    rx.close();
                },
                Ok(Ok(_)) => {}
            }
            if let Outgoing::Disconnect { .. } = packet {
                rx.close();
            }
            if rx.is_closed() {
                let mut server = server.lock().expect("other thread panicked");
                let player_state = player_state.lock().expect("other thread panicked");
                if let Some(world) = server.worlds.get_mut(&player_state.current_world) {
                    world.remove_player(player_state.id);
                }
            }
        }
    });
    
    let send_task: JoinHandle<()> = tokio::spawn(async move {
        loop {
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
                    if config.verify_users {

                    }
                }
                Incoming::SetBlock { position, state } => {

                }
                Incoming::SetLocation { location } => {

                }
                Incoming::Message { message } => {

                }
            }
        }
    });
    
    let heartbeat_task: JoinHandle<()> = tokio::spawn(async move {
        loop {
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

}
