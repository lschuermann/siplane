use crate::api::coord_runner::rest;
use async_trait::async_trait;
use std::sync::Arc;
use uuid::Uuid;

#[async_trait]
pub trait Runner: Send + Sync + 'static {
    async fn start_job(this: &Arc<Self>, job_id: Uuid, environment_id: Uuid, ssh_keys: Vec<String>);
    async fn stop_job(this: &Arc<Self>, job_id: Uuid);
}

#[async_trait]
pub trait RunnerConnector: Send + Sync + 'static {
    async fn run(&self);
    async fn post_job_state(&self, job_id: Uuid, job_state: rest::JobState);
    async fn send_job_console_log(
        &self,
        job_id: Uuid,
        offset: usize,
        next: usize,
        stdio_map: &[(rest::StdioFd, usize)],
        console_bytes: Vec<u8>,
    );
}
