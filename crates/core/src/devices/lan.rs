//! Running syncing on the network: the server, finding other devices with mDNS and syncing with
//! the paired ones every minute and whenever one of them appears.

use std::collections::HashMap;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mdns_sd::{ServiceDaemon, ServiceEvent, ServiceInfo};
use serde::Serialize;

use super::Devices;
use crate::error::AppError;

const SERVICE: &str = "_ninari._tcp.local.";
const PORT: u16 = 47832;
const EVERY: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Nearby {
    pub id: String,
    pub name: String,
    #[serde(skip)]
    pub addr: SocketAddr,
    #[serde(skip)]
    fullname: String,
}

struct Running {
    daemon: ServiceDaemon,
    ticker: tokio::task::JoinHandle<()>,
}

pub struct Lan {
    devices: Arc<Devices>,
    nearby: Arc<Mutex<HashMap<String, Nearby>>>,
    running: Mutex<Option<Running>>,
}

impl Lan {
    pub fn new(devices: Arc<Devices>) -> Arc<Self> {
        Arc::new(Self {
            devices,
            nearby: Arc::new(Mutex::new(HashMap::new())),
            running: Mutex::new(None),
        })
    }

    pub fn is_running(&self) -> bool {
        self.running.lock().unwrap().is_some()
    }

    /// Devices with ninari seen on the network right now.
    pub fn nearby(&self) -> Vec<Nearby> {
        let mut list: Vec<Nearby> = self.nearby.lock().unwrap().values().cloned().collect();
        list.sort_by(|a, b| a.name.cmp(&b.name));
        list
    }

    pub async fn start(self: &Arc<Self>) -> Result<(), AppError> {
        if self.is_running() {
            return Ok(());
        }
        // The usual port when it is free, any port otherwise: mDNS tells the others which one.
        let addr = match self
            .devices
            .serve(SocketAddr::from((Ipv4Addr::UNSPECIFIED, PORT)))
            .await
        {
            Ok(addr) => addr,
            Err(_) => {
                self.devices
                    .serve(SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)))
                    .await?
            }
        };
        let daemon = ServiceDaemon::new().map_err(|e| AppError::new("Devices", e.to_string()))?;
        let host = format!("{}.local.", self.devices.id);
        let props = [
            ("id", self.devices.id.as_str()),
            ("name", self.devices.name.as_str()),
        ];
        let info = ServiceInfo::new(
            SERVICE,
            &self.devices.id,
            &host,
            "",
            addr.port(),
            &props[..],
        )
        .map_err(|e| AppError::new("Devices", e.to_string()))?
        .enable_addr_auto();
        daemon
            .register(info)
            .map_err(|e| AppError::new("Devices", e.to_string()))?;
        let events = daemon
            .browse(SERVICE)
            .map_err(|e| AppError::new("Devices", e.to_string()))?;

        let me = self.clone();
        let rt = tokio::runtime::Handle::current();
        std::thread::spawn(move || {
            while let Ok(event) = events.recv() {
                match event {
                    ServiceEvent::ServiceResolved(info) => {
                        let Some(id) = info.get_property_val_str("id").map(str::to_string) else {
                            continue;
                        };
                        let Some(ip) = info.get_addresses_v4().into_iter().next() else {
                            continue;
                        };
                        if id == me.devices.id {
                            continue;
                        }
                        let found = Nearby {
                            name: info
                                .get_property_val_str("name")
                                .unwrap_or("ninari")
                                .to_string(),
                            addr: SocketAddr::from((ip, info.get_port())),
                            fullname: info.get_fullname().to_string(),
                            id: id.clone(),
                        };
                        let fresh = me
                            .nearby
                            .lock()
                            .unwrap()
                            .insert(id.clone(), found)
                            .is_none();
                        me.devices.changed();
                        if fresh {
                            let me = me.clone();
                            rt.spawn(async move { me.sync_one(&id).await });
                        }
                    }
                    ServiceEvent::ServiceRemoved(_, fullname) => {
                        me.nearby
                            .lock()
                            .unwrap()
                            .retain(|_, n| n.fullname != fullname);
                        me.devices.changed();
                    }
                    _ => {}
                }
            }
        });

        let me = self.clone();
        let ticker = tokio::spawn(async move {
            loop {
                tokio::time::sleep(EVERY).await;
                me.sync_all().await;
            }
        });
        *self.running.lock().unwrap() = Some(Running { daemon, ticker });
        Ok(())
    }

    pub fn stop(&self) {
        if let Some(run) = self.running.lock().unwrap().take() {
            run.ticker.abort();
            let _ = run.daemon.shutdown();
        }
        self.devices.stop_server();
        self.nearby.lock().unwrap().clear();
        self.devices.stop_pairing();
        self.devices.changed();
    }

    /// The address of a device seen on the network.
    pub fn addr_of(&self, id: &str) -> Option<SocketAddr> {
        self.nearby.lock().unwrap().get(id).map(|n| n.addr)
    }

    async fn sync_one(&self, id: &str) {
        let paired = self
            .devices
            .peers()
            .map(|p| p.iter().any(|p| p.id == id))
            .unwrap_or(false);
        if let (true, Some(addr)) = (paired, self.addr_of(id)) {
            if let Err(e) = self.devices.sync_with(id, addr).await {
                tracing::warn!("sync with {id} failed: {}", e.message);
            }
        }
    }

    /// Syncs with every paired device that is on the network; returns (titles, episodes) changed here.
    pub async fn sync_all(&self) -> (usize, usize) {
        let mut total = (0, 0);
        for peer in self.devices.peers().unwrap_or_default() {
            let Some(addr) = self.addr_of(&peer.id) else {
                continue;
            };
            match self.devices.sync_with(&peer.id, addr).await {
                Ok((t, e)) => total = (total.0 + t, total.1 + e),
                Err(e) => tracing::warn!("sync with {} failed: {}", peer.id, e.message),
            }
        }
        total
    }
}
