use std::{
    hash::{Hash, Hasher},
    sync::Arc,
    time::Duration,
};

use futures::{StreamExt, channel::mpsc};
use libp2p::{
    Multiaddr, Swarm, Transport,
    core::multiaddr::Protocol,
    gossipsub, identify, kad, mdns, noise, ping,
    swarm::{NetworkBehaviour, SwarmEvent},
    tcp, yamux,
};
use serde::{Deserialize, Serialize};
use tracing::{debug, info, warn};

use crate::{
    config::Config,
    error::AppError,
    services::secrets::{Secret, SecretId, SecretsService},
};

#[derive(Debug, Clone)]
pub enum NotifierMessage {
    Notification(String),
    Response(String),
}

#[derive(Debug)]
pub enum SecretsMessage {
    PublishSecretsRequest {
        request_id: u64,
        secret_ids: Vec<SecretId>,
    },
    PublishSecretsResponse {
        request_id: u64,
        secrets: Vec<Secret>,
    },
}

#[derive(Debug, Serialize, Deserialize, Clone)]
enum GossipMessage {
    SecretBatchRequest {
        request_id: u64,
        items: Vec<SecretId>,
    },
    SecretBatchResponse {
        request_id: u64,
        secrets: Vec<Secret>,
    },
}

/// Our top-level NetworkBehaviour, combining:
/// - Kademlia DHT
/// - identify
/// - mDNS (for local discovery)
/// - Gossipsub
/// - Ping (keepalive)
#[derive(NetworkBehaviour)]
#[behaviour(to_swarm = "AppEvent")]
pub struct AppBehaviour {
    pub kad: kad::Behaviour<kad::store::MemoryStore>,
    pub identify: identify::Behaviour,
    pub mdns: mdns::tokio::Behaviour,
    pub gossipsub: gossipsub::Behaviour,
    pub ping: ping::Behaviour,
}

/// Custom event type for all sub-behaviors.
#[derive(Debug)]
pub enum AppEvent {
    Mdns(mdns::Event),
    Identify(identify::Event),
    Gossipsub(gossipsub::Event),
    Ping(ping::Event),
    Kademlia(kad::Event),
}

impl From<mdns::Event> for AppEvent {
    fn from(e: mdns::Event) -> Self {
        AppEvent::Mdns(e)
    }
}

impl From<identify::Event> for AppEvent {
    fn from(e: identify::Event) -> Self {
        AppEvent::Identify(e)
    }
}

impl From<gossipsub::Event> for AppEvent {
    fn from(e: gossipsub::Event) -> Self {
        AppEvent::Gossipsub(e)
    }
}

impl From<ping::Event> for AppEvent {
    fn from(e: ping::Event) -> Self {
        AppEvent::Ping(e)
    }
}

impl From<kad::Event> for AppEvent {
    fn from(e: kad::Event) -> Self {
        AppEvent::Kademlia(e)
    }
}

pub struct NetworkManager {
    local_key: libp2p::identity::Keypair,
    config: Config,
    pub notifier_sender: Option<mpsc::Sender<NotifierMessage>>,
    pub secrets_sender: Option<mpsc::Sender<SecretsMessage>>,
    pub secrets_service: Arc<SecretsService>,
}

impl NetworkManager {
    pub async fn new(
        local_key: libp2p::identity::Keypair,
        config: Config,
        secrets_service: Arc<SecretsService>,
    ) -> Result<Self, AppError> {
        Ok(Self {
            local_key,
            config,
            notifier_sender: None,
            secrets_sender: None,
            secrets_service,
        })
    }

    pub async fn start(&mut self) -> Result<(), AppError> {
        let (notifier_tx, mut notifier_rx) = mpsc::channel::<NotifierMessage>(64);
        let (secrets_tx, mut secrets_rx) = mpsc::channel::<SecretsMessage>(64);

        self.notifier_sender = Some(notifier_tx);
        self.secrets_sender = Some(secrets_tx);

        let peer_id = self.local_key.public().to_peer_id();
        let message_id_fn = |message: &gossipsub::Message| {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            message.data.hash(&mut hasher);
            gossipsub::MessageId::from(hasher.finish().to_string())
        };

        // Configure Gossipsub
        let gossipsub_config = gossipsub::ConfigBuilder::default()
            .heartbeat_interval(Duration::from_secs(10))
            .validation_mode(gossipsub::ValidationMode::Strict)
            .message_id_fn(message_id_fn)
            .build()
            .map_err(|e| AppError::Network(format!("Error building gossipsub config: {e}")))?;

        // Build a Ping behavior with default config
        let ping_behavior = ping::Behaviour::new(ping::Config::new());

        // Build an mDNS behavior (disable IPv6 to avoid "No route to host" on some OSes)
        let mdns_config = mdns::Config {
            enable_ipv6: false,
            ..mdns::Config::default()
        };
        let mdns_behavior =
            mdns::tokio::Behaviour::new(mdns_config, peer_id).expect("mDNS init error");

        // Build identify
        let identify_config = identify::Config::new("/p2p/1.0.0".into(), self.local_key.public());
        let identify_behavior = identify::Behaviour::new(identify_config);

        // Build Kademlia DHT
        let store = kad::store::MemoryStore::new(peer_id);
        let mut kad_config = kad::Config::default();
        kad_config.set_parallelism(3usize.try_into().unwrap());
        let kad_behavior = kad::Behaviour::with_config(peer_id, store, kad_config);

        // Combine all behaviors
        let mut behaviour = AppBehaviour {
            kad: kad_behavior,
            identify: identify_behavior,
            mdns: mdns_behavior,
            gossipsub: gossipsub::Behaviour::new(
                gossipsub::MessageAuthenticity::Signed(self.local_key.clone()),
                gossipsub_config,
            )
            .expect("Gossipsub init error"),
            ping: ping_behavior,
        };

        // DHT in "server" mode so it responds to queries from others
        behaviour.kad.set_mode(Some(kad::Mode::Server));

        // Subscribe to a global "secrets" topic for gossip
        let secrets_topic = gossipsub::IdentTopic::new("secrets");
        behaviour.gossipsub.subscribe(&secrets_topic)?;

        // Build swarm
        let mut swarm = libp2p::SwarmBuilder::with_existing_identity(self.local_key.clone())
            .with_tokio()
            .with_tcp(
                tcp::Config::default(),
                noise::Config::new,
                yamux::Config::default,
            )?
            .with_behaviour(|_| behaviour)
            .expect("Valid behaviour")
            .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(60)))
            .build();

        // Listen on the configured addresses
        for addr_str in &self.config.network.listen_addresses {
            let addr: Multiaddr = addr_str.parse()?;
            match swarm.listen_on(addr.clone()) {
                Ok(_) => info!("Listening on {}", addr),
                Err(e) => warn!("Failed to listen on {addr}: {e}"),
            }
        }

        // Always dial any configured bootstrap peers (if any)
        for peer_addr in &self.config.network.bootstrap_peers {
            self.add_peer(&mut swarm, peer_addr).await?;
        }

        let secrets_service = Arc::clone(&self.secrets_service);
        let topic_clone = secrets_topic.clone();

        // Run swarm in a background task
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    // --- Swarm event handler ---
                    event = swarm.select_next_some() => {
                        match event {
                            SwarmEvent::Behaviour(app_event) => {
                                match app_event {
                                    AppEvent::Mdns(mdns::Event::Discovered(list)) => {
                                        for (discovered_peer, addr) in list {
                                            info!("mDNS discovered peer {discovered_peer} at {addr}");
                                            swarm.behaviour_mut().kad.add_address(&discovered_peer, addr.clone());
                                            let _ = swarm.dial(addr);
                                        }
                                    }
                                    AppEvent::Mdns(mdns::Event::Expired(list)) => {
                                        for (expired_peer, addr) in list {
                                            info!("mDNS expired peer {expired_peer} at {addr}");
                                            swarm.behaviour_mut().kad.remove_address(&expired_peer, &addr);
                                        }
                                    }
                                    AppEvent::Gossipsub(gossipsub::Event::Message {
                                        propagation_source,
                                        message_id: _,
                                        message,
                                    }) => {
                                        if let Ok(msg_str) = String::from_utf8(message.data.clone()) {
                                            if let Ok(msg) = serde_json::from_str::<GossipMessage>(&msg_str) {
                                                match msg {
                                                    GossipMessage::SecretBatchRequest { request_id, items } => {
                                                        info!("Received SecretBatchRequest from peer={propagation_source} \
                                                            with request_id={request_id}, {} item(s)", items.len());
                                                        let found = secrets_service
                                                            .handle_incoming_secret_batch_request(request_id, items)
                                                            .await;
                                                        if !found.is_empty() {
                                                            info!("Found {} matching secret(s); sending SecretBatchResponse", found.len());
                                                            let response = GossipMessage::SecretBatchResponse {
                                                                request_id,
                                                                secrets: found,
                                                            };
                                                            if let Ok(payload) = serde_json::to_string(&response) {
                                                                let _ = swarm.behaviour_mut()
                                                                    .gossipsub
                                                                    .publish(topic_clone.clone(), payload.as_bytes());
                                                            }
                                                        } else {
                                                            debug!("No secrets found for request_id={request_id}");
                                                        }
                                                    }
                                                    GossipMessage::SecretBatchResponse { request_id, secrets } => {
                                                        info!("Received SecretBatchResponse for request_id={request_id} \
                                                            with {} secrets", secrets.len());
                                                        secrets_service
                                                            .handle_incoming_secret_batch_response(request_id, secrets)
                                                            .await;
                                                    }
                                                }
                                            } else {
                                                debug!("Ignoring invalid gossip message (JSON parse failed)");
                                            }
                                        } else {
                                            debug!("Ignoring gossip message (UTF-8 decode failed)");
                                        }
                                    }
                                    AppEvent::Identify(identify::Event::Received { peer_id, info, .. }) => {
                                        debug!("Identify info from peer {peer_id}: {:?}", info);
                                    }
                                    AppEvent::Ping(ping_event) => {
                                        debug!("Ping event: {:?}", ping_event);
                                    }
                                    AppEvent::Kademlia(kad_event) => {
                                        debug!("Kademlia event: {:?}", kad_event);
                                    }
                                    e => {
                                        debug!("Unhandled event: {e:?}");
                                    }
                                }
                            }
                            SwarmEvent::NewListenAddr { address, .. } => {
                                info!("New listener on {address}");
                            }
                            SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                                info!("Connection established with {peer_id}");
                            }
                            SwarmEvent::ConnectionClosed { peer_id, .. } => {
                                info!("Connection closed with {peer_id}");
                            }
                            _ => {}
                        }
                    },

                    // --- Notifier channel handler ---
                    msg = notifier_rx.next() => {
                        if let Some(msg) = msg {
                            match msg {
                                NotifierMessage::Notification(content) => {
                                    info!("NotifierMessage: {content}");
                                }
                                NotifierMessage::Response(content) => {
                                    info!("NotifierResponse: {content}");
                                }
                            }
                        }
                    },

                    // --- Secrets channel handler ---
                    msg = secrets_rx.next() => {
                        if let Some(msg) = msg {
                            match msg {
                                SecretsMessage::PublishSecretsRequest {
                                    request_id,
                                    secret_ids
                                } => {
                                    let gossip = GossipMessage::SecretBatchRequest {
                                        request_id,
                                        items: secret_ids,
                                    };
                                    if let Ok(json) = serde_json::to_string(&gossip) {
                                        let _ = swarm.behaviour_mut()
                                            .gossipsub
                                            .publish(topic_clone.clone(), json.as_bytes());
                                    } else {
                                        debug!("Failed to serialize secret batch request");
                                    }
                                }
                                SecretsMessage::PublishSecretsResponse {
                                    request_id,
                                    secrets
                                } => {
                                    let gossip = GossipMessage::SecretBatchResponse {
                                        request_id,
                                        secrets,
                                    };
                                    if let Ok(json) = serde_json::to_string(&gossip) {
                                        let _ = swarm.behaviour_mut()
                                            .gossipsub
                                            .publish(topic_clone.clone(), json.as_bytes());
                                    } else {
                                        debug!("Failed to serialize secret batch response");
                                    }
                                }
                            }
                        }
                    },
                }
            }
        });

        Ok(())
    }

    /// Attempts to parse a Multiaddr + PeerId from `peer_addr` and dial it.
    async fn add_peer(
        &self,
        swarm: &mut Swarm<AppBehaviour>,
        peer_addr: &str,
    ) -> Result<(), AppError> {
        let addr: Multiaddr = peer_addr.parse()?;
        if let Some(Protocol::P2p(peer_id)) = addr.iter().last() {
            swarm
                .behaviour_mut()
                .kad
                .add_address(&peer_id, addr.clone());
            let _ = swarm.dial(addr);
        } else {
            warn!("Peer address {peer_addr} is missing /p2p/ segment with peer ID");
        }
        Ok(())
    }
}
