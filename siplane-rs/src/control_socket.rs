use async_trait::async_trait;
use uuid::Uuid;

#[async_trait]
pub trait Runner: Send + Sync + 'static {
    async fn ssh_keys(&self, job_id: Uuid) -> Option<Vec<String>>;
}
