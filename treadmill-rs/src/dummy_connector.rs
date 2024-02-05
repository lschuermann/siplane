use std::collections::HashMap;
use std::sync::Weak;

use async_trait::async_trait;
use log::{error, info, warn};
use uuid::Uuid;

use crate::api::coord_runner::{rest as rest_api, sse as sse_api};
use crate::connector::{Runner, RunnerConnector};

pub struct DummyRunnerConnector<R: Runner> {
    environment_id: Uuid,
    _board_id: Uuid,
    runner: Weak<R>,
}

impl<R: Runner> DummyRunnerConnector<R> {
    pub fn new(board_id: Uuid, environment_id: Uuid, runner: Weak<R>) -> Self {
        DummyRunnerConnector {
            _board_id: board_id,
            environment_id,
            runner,
        }
    }
}

#[async_trait]
impl<R: Runner> RunnerConnector for DummyRunnerConnector<R> {
    async fn run(&self) {
        // Acquire a "strong" Arc<> reference to the runner. Not holding onto a
        // strong reference beyond invocations of "run" will ensure that the
        // contained runner can be deallocated properly.
        let runner = self.runner.upgrade().unwrap();

        // Generate a job id:
        let job_id = Uuid::new_v4();

        // Immediately proceed to start the requested environment:
        let ssh_keys = vec!["mytestsshkey".to_string()];
        info!(
            "Requesting new job {}, environment: {}, ssh keys: {:?}",
            job_id, self.environment_id, &ssh_keys
        );
        R::start_job(
            &runner,
            sse_api::StartJobMessage {
                job_id,
                environment_id: self.environment_id,
                ssh_keys,
                ssh_rendezvous_servers: vec![],
                job_parameters: HashMap::new(),
                board_parameters: HashMap::new(),
                environment_parameters: HashMap::new(),
                board_environment_parameters: HashMap::new(),
            },
        )
        .await;

        // Wait for SIGINT:
        info!("Job started, waiting for CTRL+C");
        match tokio::signal::ctrl_c().await {
            Ok(()) => {
                warn!("Received CTRL+C, shutting down!");
            }
            Err(err) => {
                error!("Unable to listen for shutdown signal: {}", err);
                // we also shut down in case of error
            }
        }

        info!("Requesting job {} to stop...", job_id);
        R::stop_job(&runner, sse_api::StopJobMessage { job_id }).await;

        info!("Job has stopped, exiting DummyRunnerConnector::run. Goodbye!");
    }

    async fn post_job_state(&self, job_id: Uuid, job_state: rest_api::JobState) {
        log::info!(
            "Runner provides job state for job {}: {:?}",
            job_id,
            job_state
        );
    }

    async fn send_job_console_log(
        &self,
        job_id: Uuid,
        offset: usize,
        next: usize,
        _stdio_map: &[(rest_api::StdioFd, usize)],
        console_bytes: Vec<u8>,
    ) {
        log::debug!(
            "Runner provides console log: job {}, offset {}, next: {}, length: {}, message: {:?}",
            job_id,
            offset,
            next,
            console_bytes.len(),
            String::from_utf8_lossy(&console_bytes)
        );
    }
}
