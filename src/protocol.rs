use serde::{Deserialize, Serialize};

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
