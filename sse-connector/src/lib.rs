use async_trait::async_trait;
use eventsource_client::{Client, Event, SSE};
use futures::TryStreamExt;
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};
use uuid::Uuid;

use treadmill_rs::api::coord_runner::{rest as rest_api, sse as sse_api};
use treadmill_rs::connector::{Runner, RunnerConnector};

pub struct SSERunnerConnector<R: Runner> {
    coord_url: String,
    board_id: Uuid,
    keepalive_timeout: Duration,
    reconnect_wait: Duration,
    runner: Weak<R>,
    client: reqwest::Client,
}

impl<R: Runner> SSERunnerConnector<R> {
    pub fn new(
        coord_url: String,
        board_id: Uuid,
        runner: Weak<R>,
        keepalive_timeout: Duration,
        reconnect_wait: Duration,
    ) -> Self {
        SSERunnerConnector {
            coord_url,
            board_id,
            keepalive_timeout,
            reconnect_wait,
            runner,
            client: reqwest::Client::new(),
        }
    }

    async fn handle_sse_event(&self, ev: Event, runner: &Arc<R>) {
        use sse_api::SSEMessage;

        match ev.event_type.as_str() {
            "message" => {
                match serde_json::from_str::<SSEMessage>(&ev.data) {
                    Ok(SSEMessage::UpdateState) => {
                        // runner.send_state_update().await;
                    }

                    Ok(SSEMessage::StartJob(msg)) => {
                        R::start_job(runner, msg).await;
                    }

                    Ok(SSEMessage::StopJob(msg)) => {
                        R::stop_job(runner, msg).await;
                    }

                    Err(e) => {
                        println!("Unable to parse SSE message \"{}\": {:?}", ev.data, e);
                    }
                }
            }

            "close" => {
                println!(
                    "Server closed connection, last will and testament: {}",
                    ev.data
                );
            }

            _ => println!("Unknown event type {}!", ev.event_type),
        }
    }
}

#[async_trait]
impl<R: Runner> RunnerConnector for SSERunnerConnector<R> {
    async fn run(&self) {
        // Acquire a "strong" Arc<> reference to the runner. Not holding onto a
        // strong reference beyond invocations of "run" will ensure that the
        // contained runner can be deallocated properly.
        let runner = self.runner.upgrade().unwrap();

        loop {
            println!("(Re)connecting to server...");

            // TODO: inject token as Bearer auth
            // .header("Authorization", "Basic username:password")?
            let client = eventsource_client::ClientBuilder::for_url(&format!(
                "{}/api/runner/v0/boards/{}/sse",
                self.coord_url,
                self.board_id.to_string()
            ))
            .unwrap()
            .build();

            // Initialize the keepalive timeout by tracking the last
            // keepalive message in an `Instant`:
            let mut last_message = Instant::now();

            let mut stream = Box::pin(client.stream());

            loop {
                #[rustfmt::skip]
                tokio::select! {
                    sse_stream_element = stream.try_next() => {
                        match sse_stream_element {
                            Ok(Some(SSE::Event(ev))) => {
                                self.handle_sse_event(ev, &runner).await;
                                last_message = Instant::now();
                            }

                            Ok(Some(SSE::Comment(_))) => {
                                // Do nothing. We use comments for keep-alive messages only.
                                last_message = Instant::now();
                            }

                            Ok(None) => {
                                // Ignore empty stream elements.
                            },

                            Err(e) => {
                                println!("Error while processing SSE stream, terminating connection: {:?}", e);
                                break
                            }
                        }
                    }

                    _ = tokio::time::sleep(Duration::from_secs(1)) => {
                        // Check whether the keepalive timeout has expired:
                        if Instant::now().duration_since(last_message) > self.keepalive_timeout {
                            println!(
                                "No coordinator message for {} sec, terminating connection.",
                                self.keepalive_timeout.as_secs()
                            );
                            break
                        }
                    }
                }
            }

            println!(
                "EventSource connection closed, attempting to reconnect after {} sec...",
                self.reconnect_wait.as_secs()
            );
            tokio::time::sleep(self.reconnect_wait).await;
        }
    }

    async fn post_job_state(&self, job_id: Uuid, job_state: rest_api::JobState) {
        self.client
            .put(&format!(
                "{}/api/runner/v0/jobs/{}/state",
                self.coord_url,
                job_id.to_string()
            ))
            .json(&job_state)
            .send()
            .await
            .unwrap();
    }

    async fn send_job_console_log(
        &self,
        job_id: Uuid,
        offset: usize,
        next: usize,
        stdio_map: &[(rest_api::StdioFd, usize)],
        console_bytes: Vec<u8>,
    ) {
        self.client
            .put(&format!(
                "{}/api/runner/v0/jobs/{}/console",
                self.coord_url,
                job_id.to_string()
            ))
            .header("X-Treadmill-Console-Offset", format!("{}", offset))
            .header("X-Treadmill-Console-Next", format!("{}", next))
            .header(
                "X-Treadmill-Console-Stdiomap",
                serde_json::to_string(&stdio_map).unwrap(),
            )
            .header("Content-Type", "application/octet-stream")
            .body(console_bytes)
            .send()
            .await
            .unwrap();
    }
}
