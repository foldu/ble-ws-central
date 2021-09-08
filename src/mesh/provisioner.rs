use super::{proto::MeshAddr, Event, Node};
use std::convert::TryFrom;
use tokio::sync::mpsc;
use uuid::Uuid;
use zbus::dbus_interface;

pub struct Provisioner {
    tx: tokio::sync::mpsc::Sender<Event>,
    addr_cnt: u16,
}

fn extract_uuid(data: &[u8]) -> Option<Uuid> {
    let uuid_len = uuid::Bytes::default().len();
    if data.len() < uuid_len {
        return None;
    }
    match uuid::Bytes::try_from(&data[0..uuid_len]) {
        Ok(bytes) => Some(Uuid::from_bytes(bytes)),
        _ => {
            return None;
        }
    }
}

impl Provisioner {
    pub fn new(event_tx: mpsc::Sender<Event>) -> Self {
        Self {
            tx: event_tx,
            addr_cnt: 0x0005,
        }
    }
}

#[derive(zvariant::derive::DeserializeDict, zvariant::derive::TypeDict)]
struct ScanResultOptions {}

#[dbus_interface(name = "org.bluez.mesh.Provisioner1")]
impl Provisioner {
    fn scan_result(&mut self, rssi: i16, data: Vec<u8>, _options: ScanResultOptions) {
        match extract_uuid(&data) {
            Some(uuid) => {
                self.tx
                    .blocking_send(Event::DeviceDiscovered { uuid, rssi })
                    .unwrap();
            }
            None => tracing::error!("Could not extract uuid from scan_result"),
        }
    }

    fn request_prov_data(&mut self, unicast_count: u8) -> (super::types::NetIndex, MeshAddr) {
        // no idea how to handle multipe unicast addr requests with this interface
        assert!(unicast_count == 1);
        let addr = self.addr_cnt;
        self.addr_cnt = self.addr_cnt.checked_add(1).expect("Addr overflow");
        (super::MAIN_NET_INDEX, MeshAddr::from(addr))
    }

    fn add_node_complete(&self, uuid: &[u8], unicast_addr: u16, _unicast_addr_count: u8) {
        match extract_uuid(uuid) {
            Some(uuid) => self
                .tx
                .blocking_send(Event::NodeAdded(Ok(Node {
                    uuid,
                    unicast_addr: MeshAddr::from(unicast_addr),
                })))
                .unwrap(),
            None => {
                tracing::error!("Failed extracing uuid in add_node_complete")
            }
        }
    }

    fn add_node_failed(&self, uuid: &[u8], reason: &str) {
        match extract_uuid(uuid) {
            Some(uuid) => self
                .tx
                .blocking_send(Event::NodeAdded(Err(eyre::format_err!(
                    "Failed adding node {}: {}",
                    uuid,
                    reason
                ))))
                .unwrap(),
            None => {
                tracing::error!("Failed extracing uuid in add_node_failed")
            }
        }
    }
}
