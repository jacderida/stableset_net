// Copyright (C) 2024 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

pub mod control;
pub mod error;
pub mod node;
pub mod rpc;

pub mod safenode_manager_proto {
    tonic::include_proto!("safenode_manager_proto");
}

use crate::error::{Error, Result};
use async_trait::async_trait;
use libp2p::Multiaddr;
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    net::SocketAddr,
    path::{Path, PathBuf},
};

pub use node::{NodeService, NodeServiceData};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum ServiceStatus {
    /// The service has been added but not started for the first time
    Added,
    /// Last time we checked the service was running
    Running,
    /// The service has been stopped
    Stopped,
    /// The service has been removed
    Removed,
}

#[async_trait]
pub trait ServiceStateActions {
    fn bin_path(&self) -> PathBuf;
    fn data_dir_path(&self) -> PathBuf;
    fn log_dir_path(&self) -> PathBuf;
    fn name(&self) -> String;
    fn pid(&self) -> Option<u32>;
    async fn initialize(&mut self) -> Result<()>;
    fn status(&self) -> ServiceStatus;
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Faucet {
    pub faucet_path: PathBuf,
    pub local: bool,
    pub log_dir_path: PathBuf,
    pub pid: Option<u32>,
    pub service_name: String,
    pub status: ServiceStatus,
    pub user: String,
    pub version: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Daemon {
    pub daemon_path: PathBuf,
    pub endpoint: Option<SocketAddr>,
    pub pid: Option<u32>,
    pub service_name: String,
    pub status: ServiceStatus,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeRegistry {
    pub bootstrap_peers: Vec<Multiaddr>,
    pub daemon: Option<Daemon>,
    pub environment_variables: Option<Vec<(String, String)>>,
    pub faucet: Option<Faucet>,
    pub nodes: Vec<NodeServiceData>,
    pub save_path: PathBuf,
}

impl NodeRegistry {
    pub fn save(&self) -> Result<()> {
        let path = Path::new(&self.save_path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let json = serde_json::to_string(self)?;
        let mut file = std::fs::File::create(self.save_path.clone())?;
        file.write_all(json.as_bytes())?;
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(NodeRegistry {
                bootstrap_peers: vec![],
                daemon: None,
                environment_variables: None,
                faucet: None,
                nodes: vec![],
                save_path: path.to_path_buf(),
            });
        }

        let mut file = std::fs::File::open(path)?;
        let mut contents = String::new();
        file.read_to_string(&mut contents)?;

        // It's possible for the file to be empty if the user runs a `status` command before any
        // services were added.
        if contents.is_empty() {
            return Ok(NodeRegistry {
                bootstrap_peers: vec![],
                daemon: None,
                environment_variables: None,
                faucet: None,
                nodes: vec![],
                save_path: path.to_path_buf(),
            });
        }

        let registry = serde_json::from_str(&contents)?;
        Ok(registry)
    }
}

pub fn get_local_node_registry_path() -> Result<PathBuf> {
    let path = dirs_next::data_dir()
        .ok_or_else(|| Error::UserDataDirectoryNotObtainable)?
        .join("safe")
        .join("local_node_registry.json");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    Ok(path)
}
