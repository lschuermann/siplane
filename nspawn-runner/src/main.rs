use std::collections::HashMap;

use serde::Deserialize;
use uuid::Uuid;

#[derive(Deserialize)]
pub struct EnvironmentConfig {
    id: String,
    version: String,
}

#[derive(Deserialize)]
pub struct Config {
    coordinator_base_url: String,
    board_id: Uuid,
    environments: HashMap<String, EnvironmentConfig>,
}

#[derive(Deserialize)]
struct SSEMessage {
    #[serde(rename = "type")]
    message_type: String,
}

struct NspawnRunner {
}

impl NspawnRunner {
    pub fn new() -> Self {
	NspawnRunner {}
    }
}

async fn send_state_update(config: &Config, runner: &NspawnRunner) {
    let mut pld = HashMap::new();
    pld.insert("state", "idle");
    let client = reqwest::Client::new();
    client.put(
	&format!("{}/api/runner/v0/boards/{}/state", config.coordinator_base_url, config.board_id.to_string())
    )
	.json(&pld)
	.send()
	.await
	.unwrap();
}

async fn stream_loop(config: &Config, runner: &mut NspawnRunner) {
    use futures::{TryStreamExt};
    use eventsource_client::{Client, SSE};

    println!("(Re)connecting to server...");

    let mut client = eventsource_client::ClientBuilder::for_url(
	&format!("{}/api/runner/v0/boards/{}/sse", config.coordinator_base_url, config.board_id.to_string())
    ).unwrap()
    // TODO: inject token as Bearer auth
    // .header("Authorization", "Basic username:password")?
	.build();

    let mut stream = Box::pin(client.stream());
    while let Ok(sse_stream_element) = stream.try_next().await {
	match sse_stream_element {
	    Some(SSE::Event(ev)) => {
		match ev.event_type.as_str() {
		    "message" => {
			match serde_json::from_str::<SSEMessage>(&ev.data) {
			    Ok(m) => {
				println!("Received message of type {}", m.message_type);
				match m.message_type.as_str() {
				    "update_state" => {
					send_state_update(config, runner).await;
				    },

				    _ => {
					println!("Unknown SSE message type \"{}\"!", m.message_type);
				    }
				}
			    },

			    Err(e) => {
				println!("Unable to parse SSE message \"{}\": {:?}", ev.data, e);
			    }
			}
		    },

		    "close" => {
			println!("Server closed connection, last will and testament: {}", ev.data);
		    },

		    _ => println!("Unknown event type {}!", ev.event_type),
		}
	    },

	    Some(SSE::Comment(_)) => {
		// Do nothing. We use comments for keep-alive messages
		// only.
	    },

	    None => {
		// Ignore empty stream elements.
	    },
	}
    }

    println!("EventSource connection closed, attempting to reconnect...");
}

#[tokio::main]
async fn main() {
    let config_str = std::fs::read_to_string("runner_config.toml").unwrap();
    let config: Config = toml::from_str(&config_str).unwrap();

    let mut nspawn_runner = NspawnRunner::new();

    loop {
	stream_loop(&config, &mut nspawn_runner).await
    }
}
