use std::time::Duration;

use futures::{StreamExt, channel::mpsc};
use libp2p::{
    Multiaddr, Swarm,
    core::multiaddr::Protocol,
    identify, identity, kad, mdns, noise,
    swarm::{NetworkBehaviour, SwarmEvent},
    tcp, yamux,
};
use tracing::{debug, info, warn};

use crate::{config::Config, error::AppError};

// Message types for our services
#[derive(Debug, Clone)]
pub enum NotifierMessage {
    Notification(String),
    Response(String),
}

#[derive(Debug, Clone)]
pub enum SecretsMessage {
    Request(String),
    Response(String),
}

// Channel capacity for service communication
const CHANNEL_CAPACITY: usize = 64;

// Define our network behavior
#[derive(NetworkBehaviour)]
pub struct AppBehaviour {
    kad: kad::Behaviour<kad::store::MemoryStore>,
    identify: identify::Behaviour,
    mdns: mdns::tokio::Behaviour,
}

pub struct NetworkManager {
    local_key: identity::Keypair,
    config: Config,
    pub notifier_sender: Option<mpsc::Sender<NotifierMessage>>,
    pub secrets_sender: Option<mpsc::Sender<SecretsMessage>>,
}

impl NetworkManager {
    pub async fn new(local_key: identity::Keypair, config: Config) -> Result<Self, AppError> {
        Ok(Self {
            local_key,
            config,
            notifier_sender: None,
            secrets_sender: None,
        })
    }

    pub async fn start(&mut self) -> Result<(), AppError> {
        let peer_id = self.local_key.public().to_peer_id();

        let (notifier_sender, mut notifier_receiver) = mpsc::channel(CHANNEL_CAPACITY);
        let (secrets_sender, mut secrets_receiver) = mpsc::channel(CHANNEL_CAPACITY);

        self.notifier_sender = Some(notifier_sender.clone());
        self.secrets_sender = Some(secrets_sender.clone());

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

                AppBehaviour {
                    kad,
                    identify,
                    mdns,
                }
            })
            .expect("infallible")
            .with_swarm_config(|c| c.with_idle_connection_timeout(Duration::from_secs(60)))
            .build();

        swarm.behaviour_mut().kad.set_mode(Some(kad::Mode::Server));

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
                            Self::handle_secrets_message(&mut swarm, msg).await;
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
            SwarmEvent::Behaviour(AppBehaviourEvent::Mdns(mdns::Event::Discovered(list))) => {
                for (peer_id, addr) in list {
                    info!("mDNS discovered peer: {} at {}", peer_id, addr);
                    // Add the discovered address to Kademlia so it can be used for routing.
                    swarm
                        .behaviour_mut()
                        .kad
                        .add_address(&peer_id, addr.clone());
                    // Actively dial the discovered peer.
                    match swarm.dial(addr.clone()) {
                        Ok(_) => info!("Dialing discovered peer: {}", addr),
                        Err(e) => warn!("Failed to dial discovered peer {}: {}", addr, e),
                    }
                }
            }
            SwarmEvent::Behaviour(AppBehaviourEvent::Mdns(mdns::Event::Expired(expired_list))) => {
                for (peer_id, addr) in expired_list {
                    info!("mDNS expired peer: {} at {}", peer_id, addr);
                    swarm.behaviour_mut().kad.remove_address(&peer_id, &addr);
                }
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

    async fn handle_secrets_message(swarm: &mut Swarm<AppBehaviour>, msg: SecretsMessage) {
        match msg {
            SecretsMessage::Request(content) => {
                info!("Received secrets request: {}", content);
                // Process the request here.
            }
            SecretsMessage::Response(content) => {
                info!("Sending secrets response: {}", content);
                // Send the response to the requesting peer.
            }
        }
    }
}
