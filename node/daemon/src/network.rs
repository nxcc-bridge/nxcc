use std::{
    collections::{HashMap, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
    sync::Arc,
    time::Duration,
};

use futures::{StreamExt, channel::mpsc};
use libp2p::{
    Multiaddr, Swarm,
    core::multiaddr::Protocol,
    gossipsub, identify, kad, mdns, noise,
    swarm::{NetworkBehaviour, SwarmEvent},
    tcp, yamux,
};
use rand::Rng;
use tokio::sync::{Mutex, oneshot};
use tracing::{debug, info, warn};

use crate::{config::Config, error::AppError, services::secrets};

#[derive(Debug, Clone)]
pub enum NotifierMessage {
    Notification(String),
    Response(String),
}

#[derive(Debug)]
pub enum SecretsMessage {
    GetSecretsBatch {
        secrets: Vec<secrets::SecretIdentifier>,
        payload: Vec<u8>,
        threshold: usize,
        response_sender: oneshot::Sender<Result<Vec<secrets::BatchEncryptedSecretData>, AppError>>,
    },
    SecretBatchResponse {
        secrets: Vec<secrets::BatchEncryptedSecretData>,
    },
}

const CHANNEL_CAPACITY: usize = 64;

#[derive(NetworkBehaviour)]
pub struct AppBehaviour {
    kad: kad::Behaviour<kad::store::MemoryStore>,
    identify: identify::Behaviour,
    mdns: mdns::tokio::Behaviour,
    gossipsub: gossipsub::Behaviour,
}

pub struct NetworkManager {
    local_key: libp2p::identity::Keypair,
    config: Config,
    pub notifier_sender: Option<mpsc::Sender<NotifierMessage>>,
    pub secrets_sender: Option<mpsc::Sender<SecretsMessage>>,
    pending_batch_requests: Arc<Mutex<HashMap<u64, PendingSecretsBatch>>>,
    next_request_id: u64,
}

struct PendingSecretsBatch {
    threshold: usize,
    items: Vec<secrets::SecretIdentifier>,
    responses: Vec<secrets::BatchEncryptedSecretData>,
    response_sender: oneshot::Sender<Result<Vec<secrets::BatchEncryptedSecretData>, AppError>>,
}

impl NetworkManager {
    pub async fn new(
        local_key: libp2p::identity::Keypair,
        config: Config,
    ) -> Result<Self, AppError> {
        Ok(Self {
            local_key,
            config,
            notifier_sender: None,
            secrets_sender: None,
            pending_batch_requests: Arc::new(Mutex::new(HashMap::new())),
            next_request_id: 0,
        })
    }

    pub async fn start(&mut self) -> Result<(), AppError> {
        let peer_id = self.local_key.public().to_peer_id();
        let (notifier_sender, mut notifier_receiver) = mpsc::channel(CHANNEL_CAPACITY);
        let (secrets_sender, mut secrets_receiver) = mpsc::channel(CHANNEL_CAPACITY);

        self.notifier_sender = Some(notifier_sender.clone());
        self.secrets_sender = Some(secrets_sender.clone());

        let message_id_fn = |message: &gossipsub::Message| {
            let mut s = DefaultHasher::new();
            message.data.hash(&mut s);
            gossipsub::MessageId::from(s.finish().to_string())
        };

        let gossipsub_config = gossipsub::ConfigBuilder::default()
            .heartbeat_interval(Duration::from_secs(10))
            .validation_mode(gossipsub::ValidationMode::Strict)
            .message_id_fn(message_id_fn)
            .build()
            .map_err(|e| AppError::Network(format!("Failed to build gossipsub config: {}", e)))?;

        let mut swarm = libp2p::SwarmBuilder::with_existing_identity(self.local_key.clone())
            .with_tokio()
            .with_tcp(
                tcp::Config::default(),
                noise::Config::new,
                yamux::Config::default,
            )?
            .with_behaviour(|key| {
                let store = kad::store::MemoryStore::new(peer_id);
                let kad_config = kad::Config::default();
                let kad = kad::Behaviour::with_config(peer_id, store, kad_config);

                let identify_config = identify::Config::new("/p2p/1.0.0".to_string(), key.public());
                let identify = identify::Behaviour::new(identify_config);

                let mdns = mdns::tokio::Behaviour::new(mdns::Config::default(), peer_id)
                    .expect("Failed to create mDNS");

                let gossipsub = gossipsub::Behaviour::new(
                    gossipsub::MessageAuthenticity::Signed(key.clone()),
                    gossipsub_config,
                )
                .expect("Failed to create gossipsub");

                AppBehaviour {
                    kad,
                    identify,
                    mdns,
                    gossipsub,
                }
            })
            .expect("infallible")
            .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(60)))
            .build();

        info!("NetworkManager started; local peer_id={}", peer_id);

        swarm.behaviour_mut().kad.set_mode(Some(kad::Mode::Server));
        let secrets_topic = gossipsub::IdentTopic::new("secrets");
        swarm.behaviour_mut().gossipsub.subscribe(&secrets_topic)?;

        for addr_str in &self.config.network.listen_addresses {
            let addr: Multiaddr = addr_str.parse()?;
            match swarm.listen_on(addr.clone()) {
                Ok(_) => info!("Listening on {}", addr),
                Err(e) => warn!("Failed to listen on {}: {}", addr, e),
            }
        }

        if !self.config.network.enable_discovery {
            info!("Peer discovery disabled, adding bootstrap peers directly");
            for peer_addr in &self.config.network.bootstrap_peers {
                self.add_peer(&mut swarm, peer_addr).await?;
            }
        }

        let pending_requests = self.pending_batch_requests.clone();
        let secrets_topic_clone = secrets_topic.clone();

        tokio::spawn(async move {
            loop {
                tokio::select! {
                    event = swarm.select_next_some() => {
                        match event {
                            SwarmEvent::Behaviour(AppBehaviourEvent::Mdns(mdns::Event::Discovered(list))) => {
                                for (peer_id, addr) in list {
                                    info!("mDNS discovered peer {} at {}", peer_id, addr);
                                    swarm.behaviour_mut().kad.add_address(&peer_id, addr.clone());
                                    let _ = swarm.dial(addr);
                                }
                            }
                            SwarmEvent::Behaviour(AppBehaviourEvent::Mdns(mdns::Event::Expired(list))) => {
                                for (peer_id, addr) in list {
                                    info!("mDNS expired peer {} at {}", peer_id, addr);
                                    swarm.behaviour_mut().kad.remove_address(&peer_id, &addr);
                                }
                            }
                            SwarmEvent::Behaviour(AppBehaviourEvent::Gossipsub(gossipsub::Event::Message {
                                propagation_source,
                                message_id: _,
                                message,
                            })) => {
                                if let Ok(msg_str) = String::from_utf8(message.data.clone()) {
                                    if let Ok(msg) = serde_json::from_str::<GossipMessage>(&msg_str) {
                                        match msg {
                                            GossipMessage::SecretBatchRequest { request_id, items } => {
                                                info!("Received SecretBatchRequest from peer={} with request_id={} containing {} item(s)", propagation_source, request_id, items.len());

                                                let mut found = Vec::new();
                                                for it in items {
                                                    if has_secret(&it) {
                                                        let data = vec![1,2,3,4];
                                                        found.push(secrets::BatchEncryptedSecretData {
                                                            chain_id: it.chain_id,
                                                            identity_address: it.identity_address,
                                                            identity_id: it.identity_id,
                                                            data,
                                                            metadata: "peer share".to_string(),
                                                        });
                                                    }
                                                }
                                                if !found.is_empty() {
                                                    info!("Found {} secret(s); sending SecretBatchResponse", found.len());
                                                    let resp = GossipMessage::SecretBatchResponse {
                                                        request_id,
                                                        secrets: found,
                                                    };
                                                    if let Ok(resp_json) = serde_json::to_string(&resp) {
                                                        let _ = swarm.behaviour_mut()
                                                            .gossipsub.publish(secrets_topic_clone.clone(), resp_json.as_bytes());
                                                    }
                                                } else {
                                                    debug!("No secrets found locally for request_id={}", request_id);
                                                }
                                            }
                                            GossipMessage::SecretBatchResponse { request_id, secrets: batch } => {
                                                info!("Received SecretBatchResponse for request_id={} with {} secrets", request_id, batch.len());

                                                let mut map = pending_requests.lock().await;
                                                if let Some(req) = map.get_mut(&request_id) {
                                                    req.responses.extend(batch);
                                                    info!("Updated total responses for request_id={} to {}", request_id, req.responses.len());

                                                    if req.responses.len() >= req.threshold {
                                                        info!("Threshold met ({}/{}) for request_id={}", req.responses.len(), req.threshold, request_id);
                                                        if let Some(done) = map.remove(&request_id) {
                                                            let _ = done.response_sender.send(Ok(done.responses));
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    } else {
                                        debug!("Ignoring invalid gossip message");
                                    }
                                } else {
                                    debug!("Failed UTF-8 decode of gossip message bytes");
                                }
                            }
                            SwarmEvent::Behaviour(AppBehaviourEvent::Identify(identify::Event::Received { peer_id, info, .. })) => {
                                debug!("Identify info from peer {}: {:?}", peer_id, info);
                            }
                            SwarmEvent::NewListenAddr { address, .. } => {
                                info!("New listener on {}", address);
                            }
                            SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                                info!("Connection established with {}", peer_id);
                            }
                            SwarmEvent::ConnectionClosed { peer_id, .. } => {
                                info!("Connection closed with {}", peer_id);
                            }
                            _ => {}
                        }
                    },
                    msg = notifier_receiver.next() => {
                        if let Some(msg) = msg {
                            match msg {
                                NotifierMessage::Notification(content) => {
                                    info!("NotifierMessage: {}", content);
                                }
                                NotifierMessage::Response(content) => {
                                    info!("NotifierResponse: {}", content);
                                }
                            }
                        }
                    },
                    msg = secrets_receiver.next() => {
                        if let Some(msg) = msg {
                            match msg {
                                SecretsMessage::GetSecretsBatch {
                                    secrets: items,
                                    payload,
                                    threshold,
                                    response_sender,
                                } => {
                                    let request_id = rand::rng().random::<u64>();
                                    info!("Received SecretsMessage::GetSecretsBatch with {} items, request_id={}", items.len(), request_id);

                                    let mut map = pending_requests.lock().await;
                                    map.insert(request_id, PendingSecretsBatch {
                                        threshold,
                                        items: items.clone(),
                                        responses: Vec::new(),
                                        response_sender,
                                    });

                                    let request_msg = GossipMessage::SecretBatchRequest {
                                        request_id,
                                        items,
                                    };
                                    if let Ok(req_json) = serde_json::to_string(&request_msg) {
                                        info!("Broadcasting SecretBatchRequest for request_id={}", request_id);
                                        let publish_result = swarm.behaviour_mut()
                                            .gossipsub.publish(secrets_topic_clone.clone(), req_json.as_bytes());
                                        if let Err(e) = publish_result {
                                            warn!("Failed to publish secret batch request: {}", e);
                                            if let Some(req) = map.remove(&request_id) {
                                                let _ = req.response_sender.send(Err(AppError::Network(e.to_string())));
                                            }
                                        }
                                    }

                                    let pending_requests_clone = pending_requests.clone();
                                    tokio::spawn(async move {
                                        tokio::time::sleep(Duration::from_secs(5)).await;
                                        let mut lock = pending_requests_clone.lock().await;
                                        if let Some(req) = lock.remove(&request_id) {
                                            if !req.responses.is_empty() {
                                                info!("Time expired, but {} responses arrived for request_id={}; returning them", req.responses.len(), request_id);
                                                let _ = req.response_sender.send(Ok(req.responses));
                                            } else {
                                                info!("No responses arrived before timeout; returning fallback for request_id={}", request_id);
                                                let fallback = vec![secrets::BatchEncryptedSecretData {
                                                    chain_id: req.items[0].chain_id,
                                                    identity_address: req.items[0].identity_address,
                                                    identity_id: req.items[0].identity_id,
                                                    data: payload,
                                                    metadata: "local fallback".to_string(),
                                                }];
                                                let _ = req.response_sender.send(Ok(fallback));
                                            }
                                        }
                                    });
                                }
                                SecretsMessage::SecretBatchResponse { secrets: _ } => {
                                    info!("Received direct SecretBatchResponse, ignoring in this example");
                                }
                            }
                        }
                    },
                }
            }
        });

        Ok(())
    }

    async fn add_peer(
        &self,
        swarm: &mut Swarm<AppBehaviour>,
        peer_addr: &str,
    ) -> Result<(), AppError> {
        let addr: Multiaddr = peer_addr.parse()?;
        if let Some(Protocol::P2p(peer_id)) = addr.iter().last() {
            info!("Adding bootstrap peer {} at {}", peer_id, addr);
            swarm
                .behaviour_mut()
                .kad
                .add_address(&peer_id, addr.clone());
            let _ = swarm.dial(addr.clone());
        } else {
            warn!("Peer address {} missing /p2p/ segment with peer ID", addr);
        }
        Ok(())
    }
}

fn has_secret(_id: &secrets::SecretIdentifier) -> bool {
    // Simulate whether we have a secret to share
    rand::random()
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
enum GossipMessage {
    SecretBatchRequest {
        request_id: u64,
        items: Vec<secrets::SecretIdentifier>,
    },
    SecretBatchResponse {
        request_id: u64,
        secrets: Vec<secrets::BatchEncryptedSecretData>,
    },
}
