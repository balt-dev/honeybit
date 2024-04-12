use std::{
    sync::{Arc, Mutex},
};
use std::net::Ipv4Addr;
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
    server::Server,
    server::Config
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
            if let Err(e) = time::timeout(config.packet_timeout, packet.store(&mut write)).await {
                let _ = time::timeout(
                    config.packet_timeout,
                    Outgoing::Disconnect {
                        reason: format!("Connection error: {e}")
                    }.store(&mut write)
                ).await;
                rx.close();
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

    let _ = join!(recv_task, send_task, heartbeat_task);

}
