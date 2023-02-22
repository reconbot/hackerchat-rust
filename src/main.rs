use std::{net::{SocketAddr, Ipv4Addr}, sync::Arc};
use tokio::net::{UdpSocket}; //UdpFramed
use anyhow::{Context, Result};
// use tokio::time::sleep;
// use std::time::Duration;
use socket2::{Socket, Domain, Type, Protocol};
use async_channel::{Receiver, Sender};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;

// https://stackoverflow.com/a/35697810/339 claims 508 bytes is the only safe size for the internet
// we're not going on the internet so it's the MTU of our ethernet or maybe the max ip packet size minus some overhead so maybe 65515 bytes? People there seem to think if IP fragments our packet we're in trouble but I don't think so. Ethernet MTU is 1500 bytes? I'm going with 64kb because it's all the memory I'll ever need.
const UDP_MAX_PACKET_SIZE: usize = 64 * 1024;

struct ReceivedMessage {
    text: String,
    timestamp: u128,
    ip: String,
}

#[derive(Serialize, Deserialize)]
struct SendingMessage {
    text: String,
    timestamp: u128,
}

fn decoder(rx: Receiver<(SocketAddr, Vec<u8>)>, tx: Sender<ReceivedMessage>){
    tokio::spawn(async move {
        loop {
            let (ip, packet) = match rx.recv().await {
                Ok(data) => data,
                Err(e) => {
                    println!("Error reading rx in decoder {}", e);
                    panic!();
                }
            };
            let data = match serde_json::from_slice::<SendingMessage>(&packet) {
                Ok(data) => data,
                Err(e) => {
                    println!("Error processing JSON {}", e);
                    continue;
                }
            };

            let message = ReceivedMessage {
                text: data.text,
                timestamp: data.timestamp,
                ip: ip.ip().to_string(),
            };
            tx.send(message).await.unwrap();
        }
    });
}

fn encoder(rx: Receiver<SendingMessage>, tx: Sender<Vec<u8>>){
    tokio::spawn(async move {
        loop {
            let message = rx.recv().await.unwrap();
            let data = match serde_json::to_string(&message) {
                Ok(data) => data.as_bytes().to_vec(),
                Err(e) => {
                    println!("Error encoding JSON {}", e);
                    continue;
                }
            };
            tx.send(data).await.with_context(||"error sending encoded data").unwrap();
        }
    });
}

fn ux(rx: Receiver<(ReceivedMessage)>, tx: Sender<SendingMessage>) {
    tokio::task::spawn_blocking(move || {
        let stdin = std::io::stdin();
        loop {
            let mut text = String::new();
            stdin.read_line(&mut text).unwrap();
            let now = SystemTime::now();
            let timestamp = now.duration_since(UNIX_EPOCH).unwrap().as_millis();
            tx.send_blocking(SendingMessage { text, timestamp }).with_context(|| "send failed why?").unwrap();
        };
    });

    tokio::spawn(async move {
        loop {
            let message = rx.recv().await.unwrap();
            println!("Received Message {}", message.text);
        };
    });
}

fn network_actor(rx: Receiver<Vec<u8>>, tx: Sender<(SocketAddr, Vec<u8>)>) -> Result<()> {
    // ok so https://github.com/tokio-rs/tokio/issues/5485 means we got to do this ourselves
    // I'm not even convinced I'm allowed to do it after bind so yolo if that issue even works for us
    let socket = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    socket.set_nonblocking(true)?;
    socket.set_reuse_address(true)?;
    socket.set_reuse_port(true)?;

    let multicast_addr = "224.0.0.1".parse::<Ipv4Addr>()
        .with_context(|| "Failed to parse multicast address")?;

    socket
        .join_multicast_v4(&multicast_addr, &Ipv4Addr::UNSPECIFIED)
        .with_context(|| "Failed to join multicast group")?;

    let listen_addr = "0.0.0.0:31337".parse::<SocketAddr>()?;
    socket.bind(&listen_addr.into())
        .with_context(|| "Failed to bind to listen address")?;

    // Spawn a task to receive multicast messages
    let socket = UdpSocket::from_std(socket.into())?;
    let receiver = Arc::new(socket);
    let sender = receiver.clone();

    // receive messages
    tokio::spawn(async move {
        let mut buf = [0u8; UDP_MAX_PACKET_SIZE];
        loop {
            let (size, src) = receiver.recv_from(&mut buf).await.unwrap();
            if size > UDP_MAX_PACKET_SIZE {
                println!("ignoring a packet that's too large");
                continue;
            }
            let packet = buf[..size].to_vec();
            let _ = tx.send((src, packet));
        }
    });

    // Send a multicast message
    tokio::spawn(async move {
        let multicast_addr = "224.0.0.1:31337".parse::<SocketAddr>().unwrap();
        loop {
            let packet = rx.recv().await.with_context(||"receiving a packet to send").unwrap();
            println!("sending! {:?}", std::str::from_utf8(&packet));
            sender.send_to(&packet, &multicast_addr).await.unwrap();
        }
    });
    Ok(())
}


#[tokio::main]
async fn main() -> Result<()> {
    let num_procs = std::thread::available_parallelism()?;
    let (network_in_tx, network_in_rx) = async_channel::bounded::<(SocketAddr, Vec<u8>)>(1);
    let (network_out_tx, network_out_rx) = async_channel::bounded::<Vec<u8>>(1);
    let (ux_out_tx, ux_out_rx) = async_channel::bounded::<SendingMessage>(1);
    let (ux_in_tx, ux_in_rx) = async_channel::bounded::<ReceivedMessage>(1);

    network_actor(network_out_rx, network_in_tx)?;
    decoder(network_in_rx, ux_in_tx);
    encoder(ux_out_rx, network_out_tx);
    ux(ux_in_rx, ux_out_tx.clone());
    ux_out_tx.send(SendingMessage { text: "hey you".to_string(), timestamp: 4 }).await?;
    println!("sent the thing");
    Ok(())
}
