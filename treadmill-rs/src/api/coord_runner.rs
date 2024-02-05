pub mod sse {
    use std::collections::HashMap;

    use serde::Deserialize;
    use uuid::Uuid;

    #[derive(Deserialize, Debug, Clone)]
    pub struct ParameterValue {
        pub value: String,
        pub secret: bool,
    }

    #[derive(Deserialize, Debug, Clone)]
    #[serde(rename_all = "snake_case")]
    pub struct RendezvousServerSpec {
        pub client_id: Uuid,
        pub server_base_url: String,
        pub auth_token: String,
    }

    #[derive(Deserialize, Debug, Clone)]
    #[serde(rename_all = "snake_case")]
    pub struct StartJobMessage {
        pub job_id: Uuid,
        pub environment_id: Uuid,
        pub ssh_keys: Vec<String>,
        pub ssh_rendezvous_servers: Vec<RendezvousServerSpec>,
        pub job_parameters: HashMap<String, ParameterValue>,
        pub board_parameters: HashMap<String, ParameterValue>,
        pub environment_parameters: HashMap<String, ParameterValue>,
        pub board_environment_parameters: HashMap<String, ParameterValue>,
    }

    #[derive(Deserialize, Debug, Clone)]
    #[serde(rename_all = "snake_case")]
    pub struct StopJobMessage {
        pub job_id: Uuid,
    }

    #[derive(Deserialize, Debug, Clone)]
    #[serde(rename_all = "snake_case")]
    #[serde(tag = "type")]
    pub enum SSEMessage {
        UpdateState,
        StartJob(StartJobMessage),
        StopJob(StopJobMessage),
    }
}

pub mod rest {
    use serde::Serialize;

    #[derive(Serialize, Debug, Clone)]
    #[serde(rename_all = "snake_case")]
    pub enum JobStartingStage {
        /// Acquiring resources, such as the root file system, to launch the
        /// board environment.
        Allocating,

        /// Provisioning the environment, such as making any changes to the base
        /// system according to the user-provided customizations.
        Provisioning,

        /// The container is booting. The next transition should
        /// either be into the `Ready` or `Failed` states.
        Booting,
    }

    #[derive(Serialize, Debug, Clone)]
    #[serde(rename_all = "snake_case")]
    pub enum JobSessionConnectionInfo {
        #[serde(rename = "direct_ssh")]
        DirectSSH {
            hostname: String,
            port: u16,
            host_key_fingerprints: Vec<String>,
        },
        #[serde(rename = "rendezvous_ssh")]
        RendezvousSSH {
            hostname: String,
            port: u16,
            host_key_fingerprints: Vec<String>,
        },
    }

    #[derive(Serialize, Debug, Clone)]
    #[serde(tag = "state")]
    #[serde(rename_all = "snake_case")]
    pub enum JobState {
        Starting {
            stage: JobStartingStage,
            status_message: Option<String>,
        },
        Ready {
            connection_info: Vec<JobSessionConnectionInfo>,
            status_message: Option<String>,
        },
        Stopping {
            status_message: Option<String>,
        },
        Finished {
            status_message: Option<String>,
        },
        Failed {
            status_message: Option<String>,
        },
    }

    #[derive(Serialize, Debug, Copy, Clone)]
    #[serde(rename_all = "snake_case")]
    pub enum StdioFd {
        Stdout,
        Stderr,
    }
}
