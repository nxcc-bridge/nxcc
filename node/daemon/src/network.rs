use std::{
    collections::{HashMap, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
    sync::Arc,
    time::Duration,
};

use futures::{StreamExt, channel::mpsc, future::BoxFuture};
use libp2p::{
    Multiaddr, Swarm,
    core::multiaddr::Protocol,
    gossipsub, identify, identity, kad, mdns, noise,
    swarm::{NetworkBehaviour, SwarmEvent},
    tcp, yamux,
};
use tokio::sync::{Mutex, oneshot};
use tracing::{debug, info, warn};

use crate::{config::Config, error::AppError};

// Message types for our services
#[derive(Debug, Clone)]
pub enum NotifierMessage {
    Notification(String),
    Response(String),
}

#[derive(Debug)]
pub enum SecretsMessage {
    Request(String),
    Response(String),
    GetSecret {
        chain_id: String,
        contract_address: String,
        secret_id: String,
        payload: Vec<u8>,
        threshold: usize,
        response_sender:
            oneshot::Sender<Result<Vec<crate::services::secrets::EncryptedSecretData>, AppError>>,
    },
    SecretResponse {
        chain_id: String,
        contract_address: String,
        secret_id: String,
        secret_data: Vec<u8>,
        metadata: String,
    },
}

// Channel capacity for service communication
const CHANNEL_CAPACITY: usize = 64;

// Define our network behavior
#[derive(NetworkBehaviour)]
pub struct AppBehaviour {
    kad: kad::Behaviour<kad::store::MemoryStore>,
    identify: identify::Behaviour,
    mdns: mdns::tokio::Behaviour,
    gossipsub: gossipsub::Behaviour,
}

pub struct NetworkManager {
    local_key: identity::Keypair,
    config: Config,
    pub notifier_sender: Option<mpsc::Sender<NotifierMessage>>,
    pub secrets_sender: Option<mpsc::Sender<SecretsMessage>>,
    pending_secret_requests: Arc<Mutex<HashMap<String, PendingSecretRequest>>>,
}

struct PendingSecretRequest {
    chain_id: String,
    contract_address: String,
    secret_id: String,
    threshold: usize,
    responses: Vec<(Vec<u8>, String)>, // (data, metadata)
    response_sender:
        oneshot::Sender<Result<Vec<crate::services::secrets::EncryptedSecretData>, AppError>>,
}

impl NetworkManager {
    pub async fn new(local_key: identity::Keypair, config: Config) -> Result<Self, AppError> {
        Ok(Self {
            local_key,
            config,
            notifier_sender: None,
            secrets_sender: None,
            pending_secret_requests: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub async fn start(&mut self) -> Result<(), AppError> {
        let peer_id = self.local_key.public().to_peer_id();

        let (notifier_sender, mut notifier_receiver) = mpsc::channel(CHANNEL_CAPACITY);
        let (secrets_sender, mut secrets_receiver) = mpsc::channel(CHANNEL_CAPACITY);

        self.notifier_sender = Some(notifier_sender.clone());
        self.secrets_sender = Some(secrets_sender.clone());

        // Create a custom message ID function for gossipsub
        let message_id_fn = |message: &gossipsub::Message| {
            let mut s = DefaultHasher::new();
            message.data.hash(&mut s);
            gossipsub::MessageId::from(s.finish().to_string())
        };

        // Configure gossipsub
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
                // Set up Kademlia
                let store = kad::store::MemoryStore::new(peer_id);
                let kad_config = kad::Config::default();
                let kad = kad::Behaviour::with_config(peer_id, store, kad_config);

                // Set up identify
                let identify_config = identify::Config::new("/p2p/1.0.0".to_string(), key.public());
                let identify = identify::Behaviour::new(identify_config);

                // Set up mDNS for local peer discovery
                let mdns = mdns::tokio::Behaviour::new(mdns::Config::default(), peer_id)
                    .expect("Failed to create mDNS behavior");

                // Set up gossipsub
                let gossipsub = gossipsub::Behaviour::new(
                    gossipsub::MessageAuthenticity::Signed(key.clone()),
                    gossipsub_config,
                )
                .expect("Failed to create gossipsub behavior");

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

        swarm.behaviour_mut().kad.set_mode(Some(kad::Mode::Server));

        // Subscribe to the secrets topic
        let secrets_topic = gossipsub::IdentTopic::new("secrets");
        swarm.behaviour_mut().gossipsub.subscribe(&secrets_topic)?;

        for addr_str in &self.config.network.listen_addresses {
            let addr: Multiaddr = addr_str.parse()?;
            match swarm.listen_on(addr.clone()) {
                Ok(_) => info!("Listening on {}", addr),
                Err(e) => warn!("Failed to listen on {}: {}", addr, e),
            }
        }

        // Bootstrap with the configured peers if discovery is disabled
        if !self.config.network.enable_discovery {
            info!("Automatic peer discovery is disabled, using bootstrap peers");
            for peer_addr in &self.config.network.bootstrap_peers {
                self.add_peer(&mut swarm, peer_addr).await?;
            }
        } else {
            info!("Automatic peer discovery is enabled");
        }

        let pending_requests = self.pending_secret_requests.clone();
        let secrets_topic_clone = secrets_topic.clone();

        // Spawn the network event loop
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    event = swarm.select_next_some() => {
                        match event {
                            // When mDNS discovers peers, add their address and try dialing them.
                            SwarmEvent::Behaviour(AppBehaviourEvent::Mdns(mdns::Event::Discovered(list))) => {
                                for (peer_id, addr) in list {
                                    info!("mDNS discovered peer: {} at {}", peer_id, addr);
                                    swarm.behaviour_mut().kad.add_address(&peer_id, addr.clone());
                                    match swarm.dial(addr.clone()) {
                                        Ok(_) => info!("Dialing {}", addr),
                                        Err(e) => warn!("Failed to dial {}: {}", addr, e),
                                    }
                                }
                            },
                            // Handle expired addresses.
                            SwarmEvent::Behaviour(AppBehaviourEvent::Mdns(mdns::Event::Expired(expired_list))) => {
                                for (peer_id, addr) in expired_list {
                                    info!("mDNS expired peer: {} at {}", peer_id, addr);
                                    swarm.behaviour_mut().kad.remove_address(&peer_id, &addr);
                                }
                            },
                            // Handle gossipsub messages
                            SwarmEvent::Behaviour(AppBehaviourEvent::Gossipsub(gossipsub::Event::Message {
                                propagation_source: peer_id,
                                message_id,
                                message,
                            })) => {
                                if let Ok(msg_str) = String::from_utf8(message.data.clone()) {
                                    if let Ok(msg) = serde_json::from_str::<GossipMessage>(&msg_str) {
                                        match msg {
                                            GossipMessage::SecretRequest { chain_id, contract_address, secret_id } => {
                                                info!("Received secret request from {}: chain_id={}, contract_address={}, secret_id={}",
                                                    peer_id, chain_id, contract_address, secret_id);

                                                // In a real implementation, check if we have the secret and respond
                                                // For now, just simulate having the secret sometimes
                                                if rand::random::<bool>() {
                                                    info!("Sending secret response to {}", peer_id);
                                                    let response = GossipMessage::SecretResponse {
                                                        chain_id: chain_id.clone(),
                                                        contract_address: contract_address.clone(),
                                                        secret_id: secret_id.clone(),
                                                        secret_data: vec![1, 2, 3, 4], // Dummy data
                                                        metadata: "peer response".to_string(),
                                                    };

                                                    if let Ok(response_json) = serde_json::to_string(&response) {
                                                        if let Err(e) = swarm.behaviour_mut().gossipsub
                                                            .publish(secrets_topic_clone.clone(), response_json.as_bytes()) {
                                                            warn!("Failed to publish secret response: {}", e);
                                                        }
                                                    }
                                                } else {
                                                    info!("Don't have the requested secret");
                                                }
                                            },
                                            GossipMessage::SecretResponse { chain_id, contract_address, secret_id, secret_data, metadata } => {
                                                info!("Received secret response from {}: chain_id={}, contract_address={}, secret_id={}",
                                                    peer_id, chain_id, contract_address, secret_id);

                                                // Check if we have a pending request for this secret
                                                let request_key = format!("{}:{}:{}", chain_id, contract_address, secret_id);
                                                let mut pending_map = pending_requests.lock().await;

                                                if let Some(request) = pending_map.get_mut(&request_key) {
                                                    info!("Adding response to pending request");
                                                    request.responses.push((secret_data, metadata));

                                                    // If we've reached the threshold, complete the request
                                                    if request.responses.len() >= request.threshold {
                                                        info!("Threshold reached for secret request, completing");
                                                        if let Some(request) = pending_map.remove(&request_key) {
                                                            let secrets = request.responses.into_iter()
                                                                .map(|(data, metadata)| crate::services::secrets::EncryptedSecretData {
                                                                    data,
                                                                    metadata,
                                                                })
                                                                .collect();

                                                            let _ = request.response_sender.send(Ok(secrets));
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            },
                            // Handle all other events as before.
                            other => {
                                Self::handle_swarm_event(&mut swarm, other).await;
                            },
                        }
                    },
                    msg = notifier_receiver.next() => {
                        if let Some(msg) = msg {
                            Self::handle_notifier_message(&mut swarm, msg).await;
                        }
                    },
                    msg = secrets_receiver.next() => {
                        if let Some(msg) = msg {
                            Self::handle_secrets_message(&mut swarm, &secrets_topic_clone, &pending_requests, msg).await;
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

        // Extract peer ID if it's in the multiaddr
        if let Some(Protocol::P2p(peer_id)) = addr.iter().last() {
            info!("Adding bootstrap peer: {} at {}", peer_id, addr);
            swarm
                .behaviour_mut()
                .kad
                .add_address(&peer_id, addr.clone());

            // Try to dial the peer
            match swarm.dial(addr.clone()) {
                Ok(_) => info!("Dialing {}", addr),
                Err(e) => warn!("Failed to dial {}: {}", addr, e),
            }
        } else {
            warn!("Bootstrap peer address {} doesn't contain peer ID", addr);
        }

        Ok(())
    }

    // Updated handler now accepts a mutable reference to the swarm.
    async fn handle_swarm_event(
        swarm: &mut Swarm<AppBehaviour>,
        event: SwarmEvent<AppBehaviourEvent>,
    ) {
        match event {
            SwarmEvent::NewListenAddr { address, .. } => {
                info!("Listening on {}", address);
            }
            SwarmEvent::ConnectionEstablished { peer_id, .. } => {
                info!("Connected to {}", peer_id);
            }
            SwarmEvent::ConnectionClosed { peer_id, .. } => {
                info!("Disconnected from {}", peer_id);
            }
            SwarmEvent::Behaviour(AppBehaviourEvent::Identify(identify::Event::Received {
                peer_id,
                info,
                ..
            })) => {
                info!("Identified peer: {}", peer_id);
                debug!("  Protocol version: {}", info.protocol_version);
                debug!("  Agent version: {}", info.agent_version);
                debug!("  Observed address: {}", info.observed_addr);
                debug!("  Protocols: {:?}", info.protocols);
            }
            _ => {}
        }
    }

    async fn handle_notifier_message(swarm: &mut Swarm<AppBehaviour>, msg: NotifierMessage) {
        match msg {
            NotifierMessage::Notification(content) => {
                info!("Broadcasting notification: {}", content);
                // In a real implementation, you would send this message to connected peers.
            }
            NotifierMessage::Response(content) => {
                info!("Sending notification response: {}", content);
                // In a real implementation, you would send this response to the requesting peer.
            }
        }
    }

    async fn handle_secrets_message(
        swarm: &mut Swarm<AppBehaviour>,
        topic: &gossipsub::IdentTopic,
        pending_requests: &Arc<Mutex<HashMap<String, PendingSecretRequest>>>,
        msg: SecretsMessage,
    ) {
        match msg {
            SecretsMessage::Request(content) => {
                info!("Received secrets request: {}", content);
                // Process the request here.
            }
            SecretsMessage::Response(content) => {
                info!("Sending secrets response: {}", content);
                // Send the response to the requesting peer.
            }
            SecretsMessage::GetSecret {
                chain_id,
                contract_address,
                secret_id,
                payload,
                threshold,
                response_sender,
            } => {
                info!(
                    "Processing GetSecret request: chain_id={}, contract_address={}, \
                     secret_id={}, threshold={}",
                    chain_id, contract_address, secret_id, threshold
                );

                // Create a request key for tracking
                let request_key = format!("{}:{}:{}", chain_id, contract_address, secret_id);

                // Store the pending request
                {
                    let mut pending_map = pending_requests.lock().await;
                    pending_map.insert(
                        request_key.clone(),
                        PendingSecretRequest {
                            chain_id: chain_id.clone(),
                            contract_address: contract_address.clone(),
                            secret_id: secret_id.clone(),
                            threshold,
                            responses: Vec::new(),
                            response_sender,
                        },
                    );
                }

                // Broadcast the request to the network
                let request = GossipMessage::SecretRequest {
                    chain_id,
                    contract_address,
                    secret_id,
                };

                if let Ok(request_json) = serde_json::to_string(&request) {
                    info!("Broadcasting secret request to network");
                    if let Err(e) = swarm
                        .behaviour_mut()
                        .gossipsub
                        .publish(topic.clone(), request_json.as_bytes())
                    {
                        warn!("Failed to publish secret request: {}", e);

                        // If we can't publish, complete the request with an error
                        let mut pending_map = pending_requests.lock().await;
                        if let Some(request) = pending_map.remove(&request_key) {
                            let _ = request.response_sender.send(Err(AppError::Network(format!(
                                "Failed to publish secret request: {}",
                                e
                            ))));
                        }
                    }

                    // Set up a timeout to complete the request if we don't get enough responses
                    let pending_clone = pending_requests.clone();
                    let request_key_clone = request_key.clone();
                    tokio::spawn(async move {
                        tokio::time::sleep(Duration::from_secs(5)).await;

                        let mut pending_map = pending_clone.lock().await;
                        if let Some(request) = pending_map.remove(&request_key_clone) {
                            info!(
                                "Secret request timed out, returning {} responses",
                                request.responses.len()
                            );

                            // If we have some responses, return them, otherwise generate a local one
                            if !request.responses.is_empty() {
                                let secrets = request
                                    .responses
                                    .into_iter()
                                    .map(|(data, metadata)| {
                                        crate::services::secrets::EncryptedSecretData {
                                            data,
                                            metadata,
                                        }
                                    })
                                    .collect();

                                let _ = request.response_sender.send(Ok(secrets));
                            } else {
                                // Generate a local response
                                info!("No responses received, generating local secret");
                                let secrets = vec![crate::services::secrets::EncryptedSecretData {
                                    data: payload,
                                    metadata: "locally generated".to_string(),
                                }];

                                let _ = request.response_sender.send(Ok(secrets));
                            }
                        }
                    });
                }
            }
            SecretsMessage::SecretResponse {
                chain_id,
                contract_address,
                secret_id,
                secret_data,
                metadata,
            } => {
                info!(
                    "Received direct SecretResponse: chain_id={}, contract_address={}, \
                     secret_id={}",
                    chain_id, contract_address, secret_id
                );

                // Check if we have a pending request for this secret
                let request_key = format!("{}:{}:{}", chain_id, contract_address, secret_id);
                let mut pending_map = pending_requests.lock().await;

                if let Some(request) = pending_map.get_mut(&request_key) {
                    request.responses.push((secret_data, metadata));

                    // If we've reached the threshold, complete the request
                    if request.responses.len() >= request.threshold {
                        if let Some(request) = pending_map.remove(&request_key) {
                            let secrets = request
                                .responses
                                .into_iter()
                                .map(|(data, metadata)| {
                                    crate::services::secrets::EncryptedSecretData { data, metadata }
                                })
                                .collect();

                            let _ = request.response_sender.send(Ok(secrets));
                        }
                    }
                }
            }
        }
    }
}

// Define message types for gossipsub
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
enum GossipMessage {
    SecretRequest {
        chain_id: String,
        contract_address: String,
        secret_id: String,
    },
    SecretResponse {
        chain_id: String,
        contract_address: String,
        secret_id: String,
        secret_data: Vec<u8>,
        metadata: String,
    },
}
