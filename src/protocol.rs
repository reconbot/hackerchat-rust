use crate::network;
use serde::{Deserialize, Serialize};
use anyhow::{Context, Result};
use async_channel::{Sender, Receiver};
use std::net::{SocketAddr};

#[derive(Debug)]
pub struct ReceivedMessage {
    pub text: String,
    pub timestamp: u128,
    pub ip: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct SendingMessage {
    pub text: String,
    pub timestamp: u128,
}

pub async fn decoder(rx: Receiver<(SocketAddr, Vec<u8>)>, tx: Sender<ReceivedMessage>) -> Result<()>{
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

pub async fn encoder(rx: Receiver<SendingMessage>, tx: Sender<Vec<u8>>) -> Result<()> {
    tokio::spawn(async move {
        loop {
            let message = rx.recv().await.unwrap();
            let wrapped = textwrap::fill(&message.text, network::UDP_MAX_PACKET_SIZE - 1024);
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
