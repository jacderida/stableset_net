// Copyright (C) 2024 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under The General Public License (GPL), version 3.
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied. Please review the Licences for the specific language governing
// permissions and limitations relating to use of the SAFE Network Software.

use crate::{error::Result, rpc::RpcActions, ServiceStateActions, ServiceStatus};
use async_trait::async_trait;
use libp2p::{multiaddr::Protocol, Multiaddr, PeerId};
use serde::{de::Error as DeError, Deserialize, Deserializer, Serialize, Serializer};
use sn_protocol::get_port_from_multiaddr;
use std::{net::SocketAddr, path::PathBuf, str::FromStr};

pub struct NodeService {
    pub service_data: NodeServiceData,
    pub rpc_actions: Box<dyn RpcActions + Send>,
}

impl NodeService {
    pub fn new(
        service_data: NodeServiceData,
        rpc_actions: Box<dyn RpcActions + Send>,
    ) -> NodeService {
        NodeService {
            rpc_actions,
            service_data,
        }
    }
}

#[async_trait]
impl ServiceStateActions for NodeService {
    fn bin_path(&self) -> PathBuf {
        self.service_data.safenode_path.clone()
    }

    fn data_dir_path(&self) -> PathBuf {
        self.service_data.data_dir_path.clone()
    }

    fn log_dir_path(&self) -> PathBuf {
        self.service_data.log_dir_path.clone()
    }

    fn name(&self) -> String {
        self.service_data.service_name.clone()
    }

    fn pid(&self) -> Option<u32> {
        self.service_data.pid.clone()
    }

    async fn initialize(&mut self) -> Result<()> {
        let node_info = self.rpc_actions.node_info().await?;
        let network_info = self.rpc_actions.network_info().await?;
        self.service_data.listen_addr = Some(
            network_info
                .listeners
                .into_iter()
                .map(|addr| addr.with(Protocol::P2p(node_info.peer_id)))
                .collect(),
        );
        self.service_data.pid = Some(node_info.pid);
        self.service_data.peer_id = Some(node_info.peer_id);
        self.service_data.status = ServiceStatus::Running;
        Ok(())
    }

    fn status(&self) -> ServiceStatus {
        self.service_data.status.clone()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeServiceData {
    #[serde(
        serialize_with = "serialize_connected_peers",
        deserialize_with = "deserialize_connected_peers"
    )]
    pub connected_peers: Option<Vec<PeerId>>,
    pub data_dir_path: PathBuf,
    pub genesis: bool,
    pub listen_addr: Option<Vec<Multiaddr>>,
    pub local: bool,
    pub log_dir_path: PathBuf,
    pub number: u16,
    #[serde(
        serialize_with = "serialize_peer_id",
        deserialize_with = "deserialize_peer_id"
    )]
    pub peer_id: Option<PeerId>,
    pub pid: Option<u32>,
    pub rpc_socket_addr: SocketAddr,
    pub safenode_path: PathBuf,
    pub service_name: String,
    pub status: ServiceStatus,
    pub user: String,
    pub version: String,
}

fn serialize_peer_id<S>(value: &Option<PeerId>, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    if let Some(peer_id) = value {
        return serializer.serialize_str(&peer_id.to_string());
    }
    serializer.serialize_none()
}

fn deserialize_peer_id<'de, D>(deserializer: D) -> Result<Option<PeerId>, D::Error>
where
    D: Deserializer<'de>,
{
    let s: Option<String> = Option::deserialize(deserializer)?;
    if let Some(peer_id_str) = s {
        PeerId::from_str(&peer_id_str)
            .map(Some)
            .map_err(DeError::custom)
    } else {
        Ok(None)
    }
}

fn serialize_connected_peers<S>(
    connected_peers: &Option<Vec<PeerId>>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    match connected_peers {
        Some(peers) => {
            let peer_strs: Vec<String> = peers.iter().map(|p| p.to_string()).collect();
            serializer.serialize_some(&peer_strs)
        }
        None => serializer.serialize_none(),
    }
}

fn deserialize_connected_peers<'de, D>(deserializer: D) -> Result<Option<Vec<PeerId>>, D::Error>
where
    D: Deserializer<'de>,
{
    let vec: Option<Vec<String>> = Option::deserialize(deserializer)?;
    match vec {
        Some(peer_strs) => {
            let peers: Result<Vec<PeerId>, _> = peer_strs
                .into_iter()
                .map(|s| PeerId::from_str(&s).map_err(DeError::custom))
                .collect();
            peers.map(Some)
        }
        None => Ok(None),
    }
}

impl NodeServiceData {
    /// Returns the UDP port from our node's listen address.
    pub fn get_safenode_port(&self) -> Option<u16> {
        // assuming the listening addr contains /ip4/127.0.0.1/udp/56215/quic-v1/p2p/<peer_id>
        if let Some(multi_addrs) = &self.listen_addr {
            for addr in multi_addrs {
                if let Some(port) = get_port_from_multiaddr(addr) {
                    return Some(port);
                }
            }
        }
        None
    }
}
