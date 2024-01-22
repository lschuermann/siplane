use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[serde(tag = "type")]
#[non_exhaustive]
pub enum PuppetReq {
    Ping,
    SSHKeys,
    NetworkConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[serde(tag = "type")]
#[non_exhaustive]
pub enum PuppetEvent {
    Ready,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[serde(untagged)]
pub enum PuppetMsg {
    Event {
        puppet_event_id: u64,
        event: PuppetEvent,
    },
    Request {
        request_id: u64,
        request: PuppetReq,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[serde(tag = "type")]
#[non_exhaustive]
pub enum RunnerEvent {
    // Events:
    SSHKeysUpdatedEvent { event_id: u64 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub struct Ipv4NetworkConfig {
    pub address: std::net::Ipv4Addr,
    pub prefix_length: u8,
    pub gateway: Option<std::net::Ipv4Addr>,
    pub nameservers: Vec<std::net::Ipv4Addr>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub struct Ipv6NetworkConfig {
    pub address: std::net::Ipv6Addr,
    pub prefix_length: u8,
    pub gateway: Option<std::net::Ipv6Addr>,
    pub nameservers: Vec<std::net::Ipv6Addr>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub struct NetworkConfig {
    pub hostname: String,
    pub interface: Option<String>,
    pub ipv4: Option<Ipv4NetworkConfig>,
    pub ipv6: Option<Ipv6NetworkConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[serde(tag = "type")]
#[non_exhaustive]
pub enum RunnerResp {
    // Request reponses:
    PingResp,
    SSHKeysResp { ssh_keys: Vec<String> },
    NetworkConfig(NetworkConfig),

    // Error responses:
    UnsupportedRequest,
    JobNotFound,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[serde(untagged)]
pub enum RunnerMsg {
    Event {
        runner_event_id: u64,
        event: RunnerEvent,
    },
    Response {
        request_id: u64,
        response: RunnerResp,
    },

    // Generic error, when no more specific error applies (for
    // instance, if a message cannot be parsed at all)
    Error {
        message: String,
    },
}
