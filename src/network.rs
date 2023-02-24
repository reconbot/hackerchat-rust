use futures::{try_join};
use std::{net::{SocketAddr, Ipv4Addr}, sync::Arc};
use tokio::net::{UdpSocket};
use anyhow::{Context, Result};
use socket2::{Socket, Domain, Type, Protocol};
use async_channel::{Receiver, Sender};

// https://stackoverflow.com/a/35697810/339 claims 508 bytes is the only safe size for the internet
// we're not going on the internet so it's the MTU of our ethernet or maybe the max ip packet size minus some overhead so maybe 65515 bytes? People there seem to think if IP fragments our packet we're in trouble but I don't think so. Ethernet MTU is 1500 bytes? I'm going with 64kb because it's all the memory I'll ever need.
pub const UDP_MAX_PACKET_SIZE: usize = 64 * 1024;

pub async fn network(rx: Receiver<Vec<u8>>, tx: Sender<(SocketAddr, Vec<u8>)>) -> Result<()> {
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
