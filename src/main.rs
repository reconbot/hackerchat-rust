mod ux;
mod protocol;

use futures::{try_join, FutureExt, future::try_join_all};
use protocol::{ReceivedMessage, SendingMessage};
use std::{net::{SocketAddr, Ipv4Addr}, sync::Arc};
use tokio::net::{UdpSocket}; //UdpFramed
use anyhow::{Context, Result};
use socket2::{Socket, Domain, Type, Protocol};
use async_channel::{Receiver, Sender};

// https://stackoverflow.com/a/35697810/339 claims 508 bytes is the only safe size for the internet
// we're not going on the internet so it's the MTU of our ethernet or maybe the max ip packet size minus some overhead so maybe 65515 bytes? People there seem to think if IP fragments our packet we're in trouble but I don't think so. Ethernet MTU is 1500 bytes? I'm going with 64kb because it's all the memory I'll ever need.
const UDP_MAX_PACKET_SIZE: usize = 64 * 1024;


async fn decoder(rx: Receiver<(SocketAddr, Vec<u8>)>, tx: Sender<ReceivedMessage>) -> Result<()>{
    tokio::spawn(async move {
        loop {
            let (ip, packet) = rx.recv().await.unwrap();
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
    }).await?;
    Ok(())
}

async fn encoder(rx: Receiver<SendingMessage>, tx: Sender<Vec<u8>>) -> Result<()> {
    tokio::spawn(async move {
        loop {
            let message = rx.recv().await.unwrap();
            let wrapped = textwrap::fill(&message.text, UDP_MAX_PACKET_SIZE - 1024);
            let parts = wrapped.split("\n");
            for part in parts {
                let part_message = SendingMessage {
                    text: part.to_string(),
                    ..message
                };
                let data = match serde_json::to_string(&part_message) {
                    Ok(data) => data.as_bytes().to_vec(),
                    Err(e) => {
                        println!("Error encoding JSON {}", e);
                        continue;
                    }
                };
                tx.send(data).await.with_context(||"error sending encoded data").unwrap();
            }
        }
    });
    Ok(())
}

async fn network(rx: Receiver<Vec<u8>>, tx: Sender<(SocketAddr, Vec<u8>)>) -> Result<()> {
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
    let input = tokio::spawn(async move {
        let mut buf = [0u8; UDP_MAX_PACKET_SIZE];
        loop {
            let (size, src) = receiver.recv_from(&mut buf).await.unwrap();
            // println!("received packet! {:?}", buf[..size].to_vec());
            if size > UDP_MAX_PACKET_SIZE {
                eprintln!("ignoring a packet that's too large");
                continue;
            }
            let packet = buf[..size].to_vec();
            tx.send((src, packet)).await.unwrap();
        }
    });

    // Send a multicast message
    let output = tokio::spawn(async move {
        let multicast_addr = "224.0.0.1:31337".parse::<SocketAddr>().unwrap();
        loop {
            let packet = rx.recv().await.with_context(||"receiving a packet to send").unwrap();
            sender.send_to(&packet, &multicast_addr).await.unwrap();
            // println!("sent! {:?}", std::str::from_utf8(&packet));
        }
    });
    try_join!(input, output)?;
    Ok(())
}


#[tokio::main]
async fn main() -> Result<()> {
    let num_procs = std::thread::available_parallelism()?;
    let (network_in_tx, network_in_rx) = async_channel::bounded::<(SocketAddr, Vec<u8>)>(1);
    let (network_out_tx, network_out_rx) = async_channel::bounded::<Vec<u8>>(1);
    let (ux_out_tx, ux_out_rx) = async_channel::bounded::<SendingMessage>(1);
    let (ux_in_tx, ux_in_rx) = async_channel::bounded::<ReceivedMessage>(1);

    let mut work = vec![];

    for _ in 1..num_procs.into() {
        work.push(decoder(network_in_rx.clone(), ux_in_tx.clone()).boxed());
        work.push(encoder(ux_out_rx.clone(), network_out_tx.clone()).boxed());
    }
    work.push((network(network_out_rx, network_in_tx)).boxed());
    work.push(ux::ux(ux_in_rx, ux_out_tx.clone()).boxed());

    try_join_all(work).await?;
    Ok(())
}
