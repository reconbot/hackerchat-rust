mod ux;
mod protocol;
mod network;

use futures::{FutureExt, future::try_join_all};
use protocol::{ReceivedMessage, SendingMessage};
use std::net::{SocketAddr};
use anyhow::{Result};

#[tokio::main]
async fn main() -> Result<()> {
    let num_procs = std::thread::available_parallelism()?;
    let (network_in_tx, network_in_rx) = async_channel::bounded::<(SocketAddr, Vec<u8>)>(1);
    let (network_out_tx, network_out_rx) = async_channel::bounded::<Vec<u8>>(1);
    let (ux_out_tx, ux_out_rx) = async_channel::bounded::<SendingMessage>(1);
    let (ux_in_tx, ux_in_rx) = async_channel::bounded::<ReceivedMessage>(1);

    let mut work = vec![];

    for _ in 1..num_procs.into() {
        work.push(protocol::decoder(network_in_rx.clone(), ux_in_tx.clone()).boxed());
        work.push(protocol::encoder(ux_out_rx.clone(), network_out_tx.clone()).boxed());
    }
    work.push((network::network(network_out_rx, network_in_tx)).boxed());
    work.push(ux::ux(ux_in_rx, ux_out_tx.clone()).boxed());

    try_join_all(work).await?;
    Ok(())
}
