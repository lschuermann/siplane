use std::collections::HashMap;

#[macro_use]
extern crate serde;

use serde::Deserialize;
use uuid::Uuid;

#[derive(Deserialize)]
pub struct EnvironmentConfig {
    init: String,
}

#[derive(Deserialize)]
pub struct Config {
    coordinator_base_url: String,
    board_id: Uuid,
    environments: HashMap<String, EnvironmentConfig>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
#[serde(tag = "type")]
pub enum SSEMessage {
    UpdateState,
    StartJob {
	id: Uuid,
	environment_id: String,
	ssh_keys: Vec<String>,
    }
}

#[derive(Serialize)]
#[serde(tag = "state")]
#[serde(rename_all = "snake_case")]
enum JobState {
    Starting,
    Running,
    Stopping,
}

struct NspawnRunner<'a> {
    config: &'a Config,
    nspawn_process: Option<tokio::process::Child>,
    current_job: Uuid,
    log: Vec<String>,
    client: reqwest::Client,
}

impl<'a> NspawnRunner<'a> {
    pub fn new(config: &'a Config) -> Self {
	NspawnRunner {
	    config,
	    nspawn_process: None,
	    current_job: Uuid::nil(),
	    log: Vec::new(),
	    client: reqwest::Client::new(),
	}
    }

    pub async fn start_job(&mut self, environment_id: &str, job_id: Uuid, ssh_keys: Vec<String>) {
	println!("Starting job {:?}", job_id);
	self.client.put(
	    &format!("{}/api/runner/v0/jobs/{}/state", self.config.coordinator_base_url, job_id.to_string())
	)
	.json(&JobState::Starting)
	.send()
	.await
	.unwrap();

	if self.nspawn_process.is_some() {
	    panic!("Tried to start job with existing job already running!");
	}

	let environment_config = self.config.environments.get(environment_id).unwrap();

	let proc = tokio::process::Command::new("systemd-run")
	    .args([
		"--scope",
		"--property=DevicePolicy=closed",
		// "--property=DeviceAllow='/dev/ttyUSB1 rw'",
		"--",
		"systemd-nspawn",
		"-D",
		"/containerfs",
		"--keep-unit",
		"--private-users=pick",
		"--private-network",
		"--network-veth",
		"--bind-ro=/nix/store",
		"--bind-ro=/nix/var/nix/db",
		"--bind-ro=/nix/var/nix/daemon-socket",
		// "--bind=/dev/ttyUSB1",
		&environment_config.init,
	    ])
	    .spawn()
	    .expect("Failed to spawn environment");

	self.nspawn_process = Some(proc);
    }
}

async fn stream_loop<'a>(config: &Config, runner: &mut NspawnRunner<'a>) {
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
			    Ok(SSEMessage::UpdateState) => {
				// runner.send_state_update().await;
			    },

			    Ok(SSEMessage::StartJob { id, environment_id, ssh_keys }) => {
				runner.start_job(&environment_id, id, ssh_keys).await;
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

    let mut nspawn_runner = NspawnRunner::new(&config);

    loop {
	stream_loop(&config, &mut nspawn_runner).await;
	tokio::time::sleep(tokio::time::Duration::from_secs(5)).await;
	    
    }
}
