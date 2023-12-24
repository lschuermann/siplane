use async_trait::async_trait;
use crate::connector::{Runner, rest_api, sse_api, RunnerConnector};
use uuid::Uuid;
use log::{info, warn, debug, error};
use std::sync::{Arc, Weak};

pub struct TestRunnerConnector<R: Runner> {
    environment_id: Uuid,
    board_id: Uuid,
    runner: Weak<R>,
}

impl<R: Runner> TestRunnerConnector<R> {
    pub fn new(
        board_id: Uuid,
        environment_id: Uuid,
        runner: Weak<R>,
    ) -> Self {
        TestRunnerConnector {
            board_id,
	    environment_id,
            runner,
        }
    }
}

#[async_trait]
impl<R: Runner> RunnerConnector for TestRunnerConnector<R> {
    async fn run(&self) {
        // Acquire a "strong" Arc<> reference to the runner. Not holding onto a
        // strong reference beyond invocations of "run" will ensure that the
        // contained runner can be deallocated properly.
        let runner = self.runner.upgrade().unwrap();

	// Generate a job id:
	let job_id = Uuid::new_v4();

	// Immediately proceed to start the requested environment:
	let ssh_keys = vec![];
	info!("Requesting new job {}, environment: {}, ssh keys: {:?}", job_id, self.environment_id, &ssh_keys);
	R::start_job(&runner, job_id, self.environment_id, ssh_keys).await;

	// Wait for SIGINT:
	info!("Job started, waiting for CTRL+C");
	match tokio::signal::ctrl_c().await {
            Ok(()) => {
		error!("Received CTRL+C, shutting down!");
	    },
            Err(err) => {
		error!("Unable to listen for shutdown signal: {}", err);
		// we also shut down in case of error
            },
	}

	info!("Requesting job {} to stop...", job_id);
	R::stop_job(&runner, job_id).await;

	info!("Job has stopped, exiting TestRunnerConnector::run. Goodbye!");
    }

    async fn post_job_state(&self, job_id: Uuid, job_state: rest_api::JobState) {
	log::info!("Runner provides job state for job {}: {:?}", job_id, job_state);
    }

    async fn send_job_console_log(
        &self,
        job_id: Uuid,
        offset: usize,
        next: usize,
        stdio_map: &[(rest_api::StdioFd, usize)],
        console_bytes: Vec<u8>,
    ) {
	log::debug!("Runner provides console log: job {}, offset {}, next: {}, length: {}, message: {:?}",
		    job_id, offset, next, console_bytes.len(), String::from_utf8_lossy(&console_bytes));
    }
}
