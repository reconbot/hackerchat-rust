use std::time::{SystemTime, UNIX_EPOCH};
use chrono::{DateTime, Utc, NaiveDateTime};
use anyhow::{Context, Result};
use futures::try_join;
use crate::protocol::{SendingMessage, ReceivedMessage};
use async_channel::{Sender, Receiver};
use termion::{color, clear, cursor};
use std::io::{self, Write};

fn unix_timestamp_to_human_readable(timestamp_millis: u128) -> String {
  let datetime = DateTime::<Utc>::from_utc(
	  NaiveDateTime::from_timestamp_millis(timestamp_millis.try_into().unwrap()).unwrap(),
	  Utc,
  );
  datetime.format("%Y-%m-%d %H:%M:%S").to_string()
}

const PROMPT: &str = "> ";

pub async fn ux(rx: Receiver<ReceivedMessage>, tx: Sender<SendingMessage>) -> Result<()> {
	let input = tokio::task::spawn_blocking(move || {
		let stdin = io::stdin();
		loop {
			let mut text = String::new();
			stdin.read_line(&mut text).unwrap();
			// move cursor back up and clear line and print prompt
			print!("{}{}{}", cursor::Up(1), clear::CurrentLine, PROMPT);
			io::stdout().flush().unwrap();

			let text = text.trim_end();
			if text.is_empty() {
				continue;
			}
			let now = SystemTime::now();
			let timestamp = now.duration_since(UNIX_EPOCH).unwrap().as_millis();
			tx.send_blocking(SendingMessage { text: text.into(), timestamp }).with_context(|| "send failed why?").unwrap();
		};
	});

	let output = tokio::spawn(async move {
		loop {
			let message = rx.recv().await.unwrap();
			print!("{}{}", clear::CurrentLine, cursor::Left(65535));
			println!("{}{} {}{}: {}", color::Fg(color::LightBlack), unix_timestamp_to_human_readable(message.timestamp), message.ip, color::Fg(color::Reset), message.text);
			print!("{}", PROMPT);
			io::stdout().flush().unwrap();
		};
	});

	println!("Hacker Chat Online");
	print!("{}{}", cursor::BlinkingBar, PROMPT);
	io::stdout().flush().unwrap();

	try_join!(input, output)?;
	Ok(())
}
