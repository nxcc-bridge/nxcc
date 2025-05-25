use std::{
    collections::BTreeMap,
    hash::{Hash, Hasher},
    sync::Arc,
    time::Duration,
};

use futures::{StreamExt, channel::mpsc};
use libp2p::{
    Multiaddr, Swarm,
    core::multiaddr::Protocol,
    gossipsub, identify, kad, mdns, noise, ping,
    swarm::{NetworkBehaviour, SwarmEvent},
    tcp, yamux,
};
use nxcc_interface::types::{EnvReport, SecretId, SecretRequest, SecretsBox};
use serde::{Deserialize, Serialize};
use tracing::{debug, error, info, trace, warn};

use crate::{
    config::Config, error::AppError, grpc::enclave_client::EnclaveClient,
    services::secrets::SecretsService,
};

#[derive(Debug, Clone)]
pub enum SecretsMessage {
    PublishSecretsRequest {
        request_id: u64,
        secret_requests: BTreeMap<SecretId, Vec<SecretRequest>>,
        env_report: EnvReport,
    },
    PublishSecretsResponse {
        request_id: u64,
        secrets_box: SecretsBox,
        responder_env_report: EnvReport, // The EnvReport of the node *sending* the response
    },
}

#[derive(Debug, Serialize, Deserialize)]
enum GossipMessage {
    SecretBatchRequest {
        request_id: u64,
        secret_requests: BTreeMap<SecretId, Vec<SecretRequest>>,
        env_report: EnvReport,
    },
    SecretBatchResponse {
        request_id: u64,
        secrets_box: SecretsBox,
        responder_env_report: EnvReport, // The EnvReport of the node *sending* the response
    },
    Notification {
        content: String,
        timestamp: u64,
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
    secrets_receiver: mpsc::Receiver<SecretsMessage>,
    secrets_service: Arc<SecretsService>,
}

impl NetworkManager {
    pub async fn new(
        local_key: libp2p::identity::Keypair,
        config: Config,
        secrets_service: Arc<SecretsService>,
        secrets_receiver: mpsc::Receiver<SecretsMessage>,
    ) -> Result<Self, AppError> {
        Ok(Self {
            local_key,
            config,
            secrets_receiver,
            secrets_service,
        })
    }

    pub async fn start(
        &mut self,
        shutdown: tokio::sync::broadcast::Receiver<()>,
    ) -> Result<(), AppError> {
        let peer_id = self.local_key.public().to_peer_id();

        // Build the swarm with all behaviors
        let mut swarm = self.build_swarm(peer_id)?;

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
        let secrets_rx = std::mem::replace(&mut self.secrets_receiver, mpsc::channel(1).1);

        // Subscribe to a global "secrets" topic for gossip
        let secrets_topic = gossipsub::IdentTopic::new("secrets");

        // Run swarm in a background task
        tokio::spawn(async move {
            run_network_loop(swarm, secrets_rx, secrets_service, secrets_topic, shutdown).await;
        });

        Ok(())
    }

    fn build_swarm(&self, peer_id: libp2p::PeerId) -> Result<Swarm<AppBehaviour>, AppError> {
        // Configure message ID function for Gossipsub
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
        let mdns_behavior = mdns::tokio::Behaviour::new(mdns_config, peer_id)
            .map_err(|e| AppError::Network(format!("mDNS initialization error: {e}")))?;

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
        let swarm = libp2p::SwarmBuilder::with_existing_identity(self.local_key.clone())
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

        Ok(swarm)
    }

    /// Attempts to parse a Multiaddr + PeerId from `peer_addr` and dial it.
    async fn add_peer(
        &self,
        swarm: &mut Swarm<AppBehaviour>,
        peer_addr: &str,
    ) -> Result<(), AppError> {
        let addr: Multiaddr = peer_addr.parse()?;
        if let Some(Protocol::P2p(peer_id)) = addr.iter().last() {
            match swarm
                .behaviour_mut()
                .kad
                .add_address(&peer_id, addr.clone())
            {
                kad::RoutingUpdate::Success => {}
                kad::RoutingUpdate::Pending => {}
                kad::RoutingUpdate::Failed => {
                    return Err(AppError::Network(
                        "Failed to add address to Kademlia".to_string(),
                    ));
                }
            }
            swarm
                .dial(addr)
                .map_err(|e| AppError::Network(format!("Failed to dial: {e}")))?;
        } else {
            warn!("Peer address {peer_addr} is missing /p2p/ segment with peer ID");
        }
        Ok(())
    }
}

async fn run_network_loop(
    mut swarm: Swarm<AppBehaviour>,
    mut secrets_rx: mpsc::Receiver<SecretsMessage>,
    secrets_service: Arc<SecretsService>,
    topic: gossipsub::IdentTopic,
    mut shutdown: tokio::sync::broadcast::Receiver<()>,
) {
    loop {
        tokio::select! {
            event = swarm.select_next_some() => {
                handle_swarm_event(event, &mut swarm, &topic, &secrets_service).await;
            },

            msg = secrets_rx.next() => {
                if let Some(msg) = msg {
                    // Need secrets_service to construct the response message
                    if let Err(e) = handle_secrets_message(msg, &mut swarm, &topic, &secrets_service).await {
                        error!("Failed to handle secrets message: {e}");
                    }
                }
            },

            Ok(()) = shutdown.recv() => {
                break;
            }
        }
    }
}

async fn handle_swarm_event(
    event: SwarmEvent<AppEvent>,
    swarm: &mut Swarm<AppBehaviour>,
    topic: &gossipsub::IdentTopic,
    secrets_service: &Arc<SecretsService>,
) {
    match event {
        SwarmEvent::Behaviour(app_event) => match app_event {
            AppEvent::Mdns(mdns_event) => handle_mdns_event(mdns_event, swarm),
            AppEvent::Gossipsub(gossipsub_event) => {
                handle_gossipsub_event(gossipsub_event, swarm, topic, secrets_service).await
            }
            AppEvent::Identify(identify_event) => handle_identify_event(identify_event),
            AppEvent::Ping(ping_event) => handle_ping_event(ping_event),
            AppEvent::Kademlia(kad_event) => handle_kademlia_event(kad_event),
        },
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
}

fn handle_mdns_event(event: mdns::Event, swarm: &mut Swarm<AppBehaviour>) {
    match event {
        mdns::Event::Discovered(list) => {
            for (discovered_peer, addr) in list {
                info!("mDNS discovered peer {discovered_peer} at {addr}");
                swarm
                    .behaviour_mut()
                    .kad
                    .add_address(&discovered_peer, addr.clone());
                if let Err(e) = swarm.dial(addr) {
                    warn!("Failed to dial discovered peer: {e}");
                }
            }
        }
        mdns::Event::Expired(list) => {
            for (expired_peer, addr) in list {
                info!("mDNS expired peer {expired_peer} at {addr}");
                swarm
                    .behaviour_mut()
                    .kad
                    .remove_address(&expired_peer, &addr);
            }
        }
    }
}

async fn handle_gossipsub_event(
    event: gossipsub::Event,
    swarm: &mut Swarm<AppBehaviour>,
    topic: &gossipsub::IdentTopic,
    secrets_service: &Arc<SecretsService>,
) {
    match event {
        gossipsub::Event::Subscribed { peer_id, topic } => {
            debug!("Peer {peer_id} subscribed to topic {topic}");
        }
        gossipsub::Event::Unsubscribed { peer_id, topic } => {
            debug!("Peer {peer_id} unsubscribed from topic {topic}");
        }
        gossipsub::Event::Message {
            propagation_source,
            message_id: _,
            message,
        } => {
            handle_gossip_message(message, propagation_source, swarm, topic, secrets_service).await;
        }
        e => {
            debug!("unhandled gossip message received: {e:?}");
        }
    }
}

async fn handle_gossip_message(
    message: gossipsub::Message,
    propagation_source: libp2p::PeerId,
    swarm: &mut Swarm<AppBehaviour>,
    topic: &gossipsub::IdentTopic,
    secrets_service: &Arc<SecretsService>,
) {
    match ciborium::de::from_reader::<GossipMessage, _>(&message.data[..]) {
        Ok(msg) => match msg {
            GossipMessage::SecretBatchRequest {
                request_id,
                secret_requests,
                env_report,
            } => {
                if let Err(e) = handle_secret_batch_request(
                    request_id,
                    secret_requests,
                    env_report,
                    propagation_source,
                    swarm,
                    topic,
                    secrets_service,
                )
                .await
                {
                    error!("Error handling secret batch request: {e}");
                }
            }
            GossipMessage::SecretBatchResponse {
                request_id,
                secrets_box,
                responder_env_report,
            } => {
                info!(
                    "Received SecretBatchResponse from peer={} for request_id={}",
                    propagation_source, request_id
                );
                if let Err(e) = secrets_service
                    .handle_incoming_secret_batch_response(
                        request_id,
                        secrets_box,
                        responder_env_report,
                    )
                    .await
                {
                    error!("Error handling secrets batch response: {e:?}")
                }
            }
            GossipMessage::Notification { content, timestamp } => {
                handle_notification(content, timestamp, propagation_source);
            }
        },
        Err(e) => {
            debug!("Ignoring invalid gossip message: CBOR parse failed: {e:?}");
            if let Ok(msg_str) = String::from_utf8(message.data.clone()) {
                trace!("The message: {}", &msg_str[..msg_str.len().min(100)]);
            }
        }
    }
}

async fn handle_secret_batch_request(
    request_id: u64,
    secret_requests: BTreeMap<SecretId, Vec<SecretRequest>>,
    requester_env_report: EnvReport, // Renamed for clarity
    propagation_source: libp2p::PeerId,
    swarm: &mut Swarm<AppBehaviour>,
    topic: &gossipsub::IdentTopic,
    secrets_service: &Arc<SecretsService>,
) -> Result<(), AppError> {
    info!(
        "Received SecretBatchRequest from peer={propagation_source} with request_id={request_id}, \
         {} item(s)",
        secret_requests.len()
    );

    // SecretsService now returns the box *and* the responder's EnvReport
    let maybe_response_data = secrets_service
        .handle_incoming_secret_batch_request(
            request_id,
            secret_requests.clone(),
            requester_env_report.clone(),
        )
        .await;

    if let Some((secrets_box, responder_env_report)) = maybe_response_data {
        info!(
            "Found local secrets for request_id={request_id}, sending SecretBatchResponse to \
             network"
        );

        // The responder's EnvReport is now provided by SecretsService
        let response = GossipMessage::SecretBatchResponse {
            request_id,
            secrets_box,
            responder_env_report, // Use the report generated by SecretsService
        };

        publish_message(swarm, topic, &response)?;
    } else {
        debug!(
            "No local secrets found for request_id={request_id}, not sending response.",
            request_id = request_id
        );
    }
    Ok(())
}

fn handle_notification(content: String, timestamp: u64, propagation_source: libp2p::PeerId) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| error!("System time error: {e}"))
        .unwrap_or_default()
        .as_secs();

    info!(
        "Received gossip notification from peer={}: '{}' (sent {} seconds ago)",
        propagation_source,
        content,
        now.saturating_sub(timestamp)
    );
}

fn handle_identify_event(event: identify::Event) {
    if let identify::Event::Received { peer_id, info, .. } = event {
        debug!("Identify info from peer {peer_id}: {:?}", info);
    }
}

fn handle_ping_event(event: ping::Event) {
    debug!("Ping event: {:?}", event);
}

fn handle_kademlia_event(event: kad::Event) {
    debug!("Kademlia event: {:?}", event);
}

async fn handle_secrets_message(
    msg: SecretsMessage,
    swarm: &mut Swarm<AppBehaviour>,
    topic: &gossipsub::IdentTopic,
    _secrets_service: &Arc<SecretsService>, // May need this later for constructing responses
) -> Result<(), AppError> {
    match msg {
        SecretsMessage::PublishSecretsRequest {
            request_id,
            secret_requests,
            env_report,
        } => {
            debug!(
                "Publishing secret batch request {request_id}: {} items",
                secret_requests.len()
            );

            let gossip = GossipMessage::SecretBatchRequest {
                request_id,
                secret_requests,
                env_report,
            };

            publish_message(swarm, topic, &gossip)?;
        }
        SecretsMessage::PublishSecretsResponse {
            request_id,
            secrets_box,
            responder_env_report,
        } => {
            debug!(
                "Publishing secret batch response {request_id} (EnvReport included)",
                request_id = request_id
            );

            let gossip = GossipMessage::SecretBatchResponse {
                // Ensure field name matches GossipMessage definition
                request_id,
                secrets_box,
                responder_env_report,
            };

            publish_message(swarm, topic, &gossip)?;
        }
    }
    Ok(())
}

// Helper function to serialize and publish a message
fn publish_message<T: serde::Serialize>(
    swarm: &mut Swarm<AppBehaviour>,
    topic: &gossipsub::IdentTopic,
    message: &T,
) -> Result<(), AppError> {
    let mut buffer = Vec::new();
    ciborium::ser::into_writer(message, &mut buffer)
        .map_err(|e| AppError::Network(format!("Failed to serialize message: {e}")))?;

    swarm
        .behaviour_mut()
        .gossipsub
        .publish(topic.clone(), buffer)
        .map_err(|e| AppError::Network(format!("Failed to publish message: {e}")))?;

    debug!("Successfully published message to gossip network");
    Ok(())
}
