mod message_queue;
mod proto;
mod provisioner;
mod proxies;
mod types;

use crate::{
    mesh::{
        message_queue::{DevFilter, MessageQueue},
        proto::sig::ConfigModelAppStatus,
    },
    sensor::{SensorState, SensorValues},
    util::Hex,
};
use eyre::Context;
use proto::{MeshAddr, ToPacket};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    convert::{TryFrom, TryInto},
    sync::Arc,
    time::Duration,
};
use tokio::{
    runtime::Handle,
    sync::{mpsc, RwLock},
};
use types::BluezToken;
use uuid::Uuid;
use zbus::dbus_interface;
use zvariant::{ObjectPath, OwnedObjectPath, OwnedValue, Value};

use self::{
    message_queue::{Filter, GenericMessage},
    proto::{vendor::BleWsSensorValues, Packet, ParsePacket},
    types::{AppIndex, NetIndex},
};

// https://git.kernel.org/pub/scm/bluetooth/bluez.git/tree/doc/mesh-api.txt
// OP_VENDOR_READ_SENSOR == 12584433 0xc005f1

struct VendorModel {
    company_id: u16,
    id: u16,
    opts: HashMap<String, Value<'static>>,
}

impl VendorModel {
    fn to_dbus_obj(self) -> (u16, u16, HashMap<String, Value<'static>>) {
        (self.company_id, self.id, self.opts)
    }
}

pub(crate) const COMPANY_ID: u16 = 0x05f1;
pub(crate) const MAIN_APP_INDEX: AppIndex = AppIndex(0);
pub(crate) const MAIN_NET_INDEX: NetIndex = NetIndex(0);

pub(crate) fn listen(handle: Handle) -> Result<Box<crate::UpdateSource>, zbus::Error> {
    let (tx, rx) = mpsc::channel(1);
    let mesh_thread = std::thread::spawn(move || -> Result<(), zbus::Error> {
        let cxn = zbus::Connection::system()?;
        let mut srv = zbus::ObjectServer::new(&cxn).request_name("li._5kw.BleWsCentral")?;

        let message_queue = MessageQueue::new(&handle);

        let cxn = zbus::azync::Connection::from(cxn);
        let (event_tx, event_rx) = mpsc::channel(1);
        let (join_tx, join_rx) = mpsc::channel(1);
        let mesh_service = Arc::new(MeshService {
            company_id: 0x0000,
            product_id: 0x0000,
            version_id: 0x0000,
            capabilities: Vec::new(),
            models: Vec::new(),
            vendor_models: vec![
                VendorModel {
                    // this is correct
                    company_id: COMPANY_ID,
                    id: 0x0000,
                    opts: HashMap::new(),
                }
                .to_dbus_obj(),
                VendorModel {
                    company_id: COMPANY_ID,
                    id: 0x0001,
                    opts: HashMap::new(),
                }
                .to_dbus_obj(),
            ],
        });
        srv.at(MAIN_PATH, ObjectManager(mesh_service.to_managed_objects()))?;
        srv.at(
            APPLICATION_PATH,
            MeshApp {
                service: mesh_service.clone(),
                cxn: cxn.clone(),
                handle: handle.clone(),
                join_tx,
            },
        )?;
        srv.at(
            AGENT_PATH,
            ProvisionAgent {
                service: mesh_service.clone(),
            },
        )?;
        srv.at(
            ELEMENT_PATH,
            Element {
                service: mesh_service.clone(),
                queue: message_queue.clone(),
            },
        )?;
        srv.at(
            APPLICATION_PATH,
            provisioner::Provisioner::new(event_tx.clone()),
        )?;

        let uuid = Uuid::parse_str(SERVICE_UUID).unwrap();
        handle.spawn(async move {
            let mut mesh_loop = LoopCtx {
                cxn,
                uuid,
                tx,
                event_rx,
                join_rx,
                queue: message_queue.clone(),
            };
            let node_path = {
                let mut max_timeouts = 5;
                loop {
                    match try_attach(&mut mesh_loop).await {
                        Ok(node_path) => break node_path,
                        Err(e) => {
                            match e {
                                // systemd service needs time to start
                                zbus::Error::MethodError(name, _, _)
                                    if name.as_str() == "org.freedesktop.DBus.Error.NotFound" =>
                                {
                                        max_timeouts -= 1;
                                        if max_timeouts == 0 {
                                            panic!("Too many timeouts while trying to connect to org.bluez.mesh");
                                        }
                                    tokio::time::sleep(Duration::from_secs(1)).await;
                                }
                                _ => {
                                    tracing::error!("Could not create mesh network: {}", e);
                                    tokio::time::sleep(Duration::from_secs(5)).await;
                                }
                            }
                        }
                    }
                }
            };
            tracing::info!("Created network {}", mesh_loop.uuid);

            loop {
                if let Err(e) = mesh_loop.run(&node_path).await {
                    tracing::error!("Mesh loop failed {}", e);
                }
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        });

        loop {
            match srv.try_handle_next() {
                Ok(_) => (),
                Err(e) => {
                    tracing::error!("{}", e);
                }
            }
        }
    });

    std::thread::spawn(move || println!("{:#?}", mesh_thread.join()));

    Ok(Box::new(tokio_stream::wrappers::ReceiverStream::new(rx)))
}

struct LoopCtx {
    cxn: zbus::azync::Connection,
    uuid: Uuid,
    tx: mpsc::Sender<crate::UpdateItem>,
    event_rx: mpsc::Receiver<Event>,
    join_rx: mpsc::Receiver<Result<OwnedObjectPath, eyre::Error>>,
    queue: MessageQueue,
}

impl LoopCtx {
    async fn run(&mut self, node_path: &ObjectPath<'_>) -> Result<(), eyre::Error> {
        let mut discovering = BTreeSet::new();
        let connected: BTreeMap<MeshAddr, Uuid> = BTreeMap::new();
        let connected = Arc::new(RwLock::new(connected));
        let management_proxy = proxies::AsyncManagement1Proxy::builder(&self.cxn)
            .path(node_path)?
            .build()
            .await?;

        let node_proxy = proxies::AsyncNode1Proxy::builder(&self.cxn)
            .path(node_path)?
            .build()
            .await?;

        management_proxy
            .unprovisioned_scan(&Default::default())
            .await?;

        let mut interval = tokio::time::interval(Duration::from_secs(30));

        let mut sensor_values = self
            .queue
            .subscribe(
                Filter::default()
                    .opcode(proto::vendor::BleWsSensorValues::opcode())
                    .app_index(MAIN_APP_INDEX),
            )
            .await;

        let (trigger_tx, mut trigger_rx) = mpsc::channel(1);
        let tx = self.tx.clone();
        tokio::task::spawn({
            let connected = connected.clone();
            async move {
                while let Some(()) = trigger_rx.recv().await {
                    let mut batch = BTreeMap::new();
                    let batch_timeout = tokio::time::sleep(Duration::from_secs(3));
                    tokio::pin!(batch_timeout);
                    let connected = connected.read().await;
                    let mut timedout = connected.values().copied().collect::<BTreeSet<_>>();
                    loop {
                        tokio::select! {
                            _ = &mut batch_timeout => {
                                // TODO: notify missed updates from connected sensors
                                break;
                            }
                            Some(msg) = sensor_values.recv() => {
                                let addr = msg.source_addr;
                                let id = *connected.get(&addr).expect("Race condition in getting uuids from connected");
                                match convert_msg_to_values(msg) {
                                    Ok(values) => {
                                        timedout.remove(&id);
                                        batch.insert(id, SensorState::Connected(values));
                                        if timedout.is_empty() {
                                            break;
                                        }
                                    }
                                    Err(e) => {
                                        tracing::error!(%addr, device_id = %id, "Received invalid msg: {}", e);
                                    }
                                }
                            }
                        }
                    }
                    for id in timedout {
                        if !batch.contains_key(&id) {
                            batch.insert(id, SensorState::Unconnected);
                        }
                    }
                    if let Err(_) = tx.send(batch).await {
                        break;
                    }
                }
            }
        });

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    // FIXME: don't copy
                    trigger_tx.send(()).await.expect("Collection task died");
                    let connected = connected.read().await;
                    for (&addr, _) in &*connected {
                        tracing::info!(%addr, "Send sensor read");
                        node_proxy
                            .send(
                                &ObjectPath::try_from(ELEMENT_PATH).unwrap(),
                                addr,
                                MAIN_APP_INDEX,
                                &Default::default(),
                                proto::vendor::BleWsReadSensor.encode(),
                            )
                            .await?;
                    }
                }
                Some(event) = self.event_rx.recv() => {
                    match event {
                        Event::DeviceDiscovered { uuid, rssi } => {
                            if discovering.insert(uuid) {
                                tracing::info!(%uuid, rssi, "Discovered device");
                                management_proxy
                                    .add_node(uuid.as_bytes(), HashMap::new())
                                    .await?;
                            }
                        }
                        Event::NodeAdded(Ok(Node { uuid, unicast_addr })) => {
                            discovering.remove(&uuid);
                            // NOTE: only call after node provisioning or bluez deadlocks
                            management_proxy
                                .unprovisioned_scan(&Default::default())
                                .await?;
                            tracing::info!(%uuid, %unicast_addr, "Node added");
                            node_proxy
                                .add_app_key(
                                    &ObjectPath::try_from(ELEMENT_PATH).unwrap(),
                                    unicast_addr,
                                    MAIN_APP_INDEX,
                                    MAIN_NET_INDEX,
                                    false,
                                )
                                .await?;

                            let dev_filter = DevFilter::default().source_addr(unicast_addr);
                            let (_, msg) = self.queue.pull_msg::<proto::sig::ConfigAppKeyStatus, _>(dev_filter).await.unwrap();
                            if !msg.status.is_ok() {
                                tracing::error!(?msg.status, "Could not add app key");
                            }


                            node_proxy
                                .dev_key_send(
                                    &ObjectPath::try_from(ELEMENT_PATH).unwrap(),
                                    unicast_addr,
                                    true,
                                    MAIN_NET_INDEX,
                                    &Default::default(),
                                    proto::sig::ConfigModelAppBind {
                                        element_address: unicast_addr,
                                        app_key_index: 0,
                                        model_identifier: proto::ModelIdentifier::Vendor {
                                            model_id: 0x0000,
                                            company_id: COMPANY_ID,
                                        }
                                    }
                                    .encode(),
                                )
                                .await?;
                            let (_, msg) = self.queue.pull_msg::<proto::sig::ConfigModelAppStatus, _>(dev_filter).await.unwrap();
                            if !msg.status.is_ok() {
                                tracing::error!(?msg.model_identifier, ?msg.status, "Could not bind app key");
                            }

                            let mut connected = connected.write().await;
                            connected.insert(unicast_addr, uuid);
                            tracing::info!(%uuid, %unicast_addr, "Bound app key to model");
                        }
                        Event::NodeAdded(Err(e)) => {
                            tracing::error!("{}", e);
                        }
                    }
                }
            }
        }
    }
}

async fn try_attach(ctx: &mut LoopCtx) -> Result<OwnedObjectPath, zbus::Error> {
    let proxy = proxies::AsyncNetwork1Proxy::new(&ctx.cxn).await?;
    // FIXME: join instead when already created
    proxy
        .create_network(
            &ObjectPath::try_from(MAIN_PATH).unwrap(),
            &ctx.uuid.as_bytes()[..],
        )
        .await?;
    // proxy
    //     .join(
    //         &ObjectPath::try_from(MAIN_PATH).unwrap(),
    //         &ctx.uuid.as_bytes()[..],
    //     )
    // //     .await?;
    // tracing::info!("Joined");

    // FIXME:
    let node_path = ctx.join_rx.recv().await.unwrap().unwrap();
    let proxy = proxies::AsyncManagement1Proxy::builder(&ctx.cxn)
        .path(&node_path)?
        .build()
        .await?;
    let node_proxy = proxies::AsyncNode1Proxy::builder(&ctx.cxn)
        .path(&node_path)?
        .build()
        .await?;

    proxy.create_app_key(MAIN_NET_INDEX, MAIN_APP_INDEX).await?;

    let addr = node_proxy.addresses().await?[0];
    node_proxy
        .add_app_key(
            &ObjectPath::try_from(ELEMENT_PATH).unwrap(),
            addr,
            MAIN_APP_INDEX,
            MAIN_NET_INDEX,
            false,
        )
        .await?;

    node_proxy
        .dev_key_send(
            &ObjectPath::try_from(ELEMENT_PATH).unwrap(),
            MeshAddr::from(0x0001),
            true,
            MAIN_NET_INDEX,
            &Default::default(),
            proto::sig::ConfigModelAppBind {
                element_address: MeshAddr::from(0x0001),
                app_key_index: 0,
                model_identifier: proto::ModelIdentifier::Vendor {
                    model_id: 0x0000,
                    company_id: COMPANY_ID,
                },
            }
            .encode(),
        )
        .await?;

    let (_, msg) = ctx
        .queue
        .pull_msg::<ConfigModelAppStatus, _>(DevFilter::default())
        .await
        .unwrap();

    if !msg.status.is_ok() {
        tracing::error!(?msg.model_identifier, ?msg.status, "Could not bind app key");
    }

    Ok(node_path)
}

const MAIN_PATH: &str = "/li/_5kw/BleWsCentral";
const APPLICATION_PATH: &str = "/li/_5kw/BleWsCentral/application";
const AGENT_PATH: &str = "/li/_5kw/BleWsCentral/agent";
const ELEMENT_PATH: &str = "/li/_5kw/BleWsCentral/ele00";

const SERVICE_UUID: &str = "7182c51d-a997-41ad-99cf-0d7ebc001646";

struct MeshService {
    company_id: u16,
    product_id: u16,
    version_id: u16,
    capabilities: Vec<String>,
    models: Vec<(u16, HashMap<String, zvariant::Value<'static>>)>,
    vendor_models: Vec<(u16, u16, HashMap<String, zvariant::Value<'static>>)>,
}

impl MeshService {
    fn to_managed_objects(&self) -> ManagedObjects {
        let mut ret = HashMap::with_capacity(3);
        ret.insert(OwnedObjectPath::try_from(APPLICATION_PATH).unwrap(), {
            let mut ret = HashMap::with_capacity(2);
            ret.insert("org.bluez.mesh.Application1".to_string(), {
                let mut ret = HashMap::new();
                ret.insert("CompanyID".to_string(), self.company_id.into());
                ret.insert("ProductID".to_string(), self.product_id.into());
                ret.insert("VersionID".to_string(), self.version_id.into());
                ret
            });
            ret.insert("org.bluez.mesh.Provisioner1".to_string(), HashMap::new());
            ret
        });
        ret.insert(OwnedObjectPath::try_from(AGENT_PATH).unwrap(), {
            let mut ret = HashMap::with_capacity(1);
            ret.insert("org.bluez.mesh.ProvisionAgent1".to_owned(), {
                let mut ret = HashMap::with_capacity(1);
                ret.insert(
                    "Capabilities".to_owned(),
                    OwnedValue::from(Value::from(&self.capabilities)),
                );
                ret
            });
            ret
        });
        ret.insert(OwnedObjectPath::try_from(ELEMENT_PATH).unwrap(), {
            let mut ret = HashMap::with_capacity(1);
            ret.insert("org.bluez.mesh.Element1".to_string(), {
                let mut ret = HashMap::new();
                ret.insert(
                    "Models".to_string(),
                    OwnedValue::from(Value::from(&self.models)),
                );
                ret.insert(
                    "VendorModels".to_string(),
                    OwnedValue::from(Value::from(&self.vendor_models)),
                );
                ret.insert("Index".to_string(), 0_u8.into());
                ret
            });
            ret
        });
        ret
    }
}

type ManagedObjects = HashMap<OwnedObjectPath, HashMap<String, HashMap<String, OwnedValue>>>;

struct ObjectManager(ManagedObjects);

#[dbus_interface(name = "org.freedesktop.DBus.ObjectManager")]
impl ObjectManager {
    fn get_managed_objects(&self) -> &ManagedObjects {
        &self.0
    }

    #[dbus_interface(signal)]
    fn interfaces_added(&self, ctx: &zbus::SignalContext) -> Result<(), zbus::Error>;

    #[dbus_interface(signal)]
    fn interfaces_removed(&self, ctx: &zbus::SignalContext) -> Result<(), zbus::Error>;
}

struct MeshApp {
    service: Arc<MeshService>,
    cxn: zbus::azync::Connection,
    join_tx: mpsc::Sender<Result<OwnedObjectPath, eyre::Error>>,
    handle: tokio::runtime::Handle,
}

#[dbus_interface(name = "org.bluez.mesh.Application1")]
impl MeshApp {
    #[dbus_interface(name = "CompanyID", property)]
    fn company_id(&self) -> u16 {
        self.service.company_id
    }

    #[dbus_interface(name = "ProductID", property)]
    fn product_id(&self) -> u16 {
        self.service.product_id
    }

    #[dbus_interface(name = "VersionID", property)]
    fn version_id(&self) -> u16 {
        self.service.version_id
    }

    fn join_complete(&mut self, token: BluezToken) {
        let cxn = self.cxn.clone();
        let tx = self.join_tx.clone();
        self.handle.spawn(async move {
            let network1_proxy = proxies::AsyncNetwork1Proxy::new(&cxn).await.unwrap();

            match network1_proxy
                .attach(&ObjectPath::try_from(MAIN_PATH).unwrap(), token)
                .await
                .context("Failed attaching")
            {
                Ok((node_path, _)) => {
                    tx.send(Ok(node_path)).await.unwrap();
                }
                Err(e) => {
                    tx.send(Err(e)).await.unwrap();
                }
            }
        });
    }

    fn join_failed(&mut self, msg: String) {
        self.join_tx
            .blocking_send(Err(eyre::eyre!("Failed joining: {}", msg)))
            .unwrap();
    }
}

struct ProvisionAgent {
    service: Arc<MeshService>,
}

#[derive(zbus::DBusError, Debug)]
#[dbus_error(prefix = "li._5kw.BleWsCentral")]
enum Error {
    ZBus(zbus::Error),
    Unsupported,
}

#[dbus_interface(name = "org.bluez.mesh.ProvisionAgent1")]
impl ProvisionAgent {
    fn private_key(&self) -> Vec<u8> {
        unimplemented!()
    }

    fn public_key(&self) -> Vec<u8> {
        unimplemented!()
    }

    // TODO: make these return errors
    fn display_string(&self, _value: String) {}

    fn display_numeric(&self, _type_: String, _number: u32) {}

    fn prompt_numeric(&self, _type_: String) -> u32 {
        42
    }

    fn prompt_static(&self, _type_: String) -> Vec<u8> {
        vec![0; 16]
    }

    fn cancel(&self) {}

    #[dbus_interface(property)]
    fn capabilities(&self) -> &[String] {
        &self.service.capabilities
    }
}

struct Element {
    service: Arc<MeshService>,
    queue: message_queue::MessageQueue,
}

#[dbus_interface(name = "org.bluez.mesh.Element1")]
impl Element {
    fn message_received(
        &self,
        source_addr: MeshAddr,
        key_index: AppIndex,
        destination: zvariant::Value<'_>,
        data: &[u8],
    ) {
        let destination = if let Some(addr) = destination.downcast_ref::<u16>() {
            MeshAddr::from(*addr).to_string()
        } else if let Some(virt_addr) = destination.downcast::<Vec<u8>>() {
            Hex(&virt_addr).to_string()
        } else {
            panic!("Bluez sent garbage address")
        };
        let msg = Hex(data);
        tracing::debug!(%source_addr, %key_index, %destination, %msg, "Message received");

        match proto::Message::parse(data) {
            Ok(msg) => {
                self.queue.push_msg(
                    msg,
                    message_queue::MessageMeta::Normal {
                        source_addr,
                        app_index: key_index,
                    },
                );
            }
            Err(e) => {
                tracing::error!(error = %e, "Received invalid message");
            }
        }
    }

    fn dev_key_message_received(
        &self,
        source_addr: MeshAddr,
        remote: bool,
        net_index: NetIndex,
        data: Vec<u8>,
    ) {
        let msg = Hex(&data);
        tracing::debug!(
            %source_addr,
            %net_index,
            remote,
            %msg,
            "Dev key message received"
        );

        match proto::Message::parse(&data) {
            Ok(message) => {
                self.queue.push_msg(
                    message,
                    message_queue::MessageMeta::Device {
                        source_addr,
                        net_index,
                    },
                );
            }
            Err(e) => {
                tracing::error!(error = %e, "Received invalid message");
            }
        }
    }

    #[dbus_interface(property)]
    fn index(&self) -> u8 {
        0
    }

    fn update_model_configuration(&self, model_id: u16, config: HashMap<String, zvariant::Value>) {
        tracing::info!(model_id, ?config, "Tried to update model configuration");
    }

    #[dbus_interface(property)]
    fn vendor_models(&self) -> &[(u16, u16, HashMap<String, zvariant::Value>)] {
        &self.service.vendor_models
    }

    #[dbus_interface(property)]
    fn models(&self) -> &[(u16, HashMap<String, zvariant::Value>)] {
        &self.service.models
    }
}

fn convert_msg_to_values(msg: GenericMessage) -> Result<SensorValues, eyre::Error> {
    let timestamp = msg.received;

    let msg = BleWsSensorValues::parse_payload(&msg.content.payload)
        .ok_or_else(|| eyre::format_err!("Invalid BleWsSensorValues payload"))?;

    Ok(SensorValues {
        timestamp,
        temperature: msg.temperature.try_into()?,
        pressure: msg.pressure.into(),
        humidity: msg.humidity.try_into()?,
    })
}

#[derive(Debug)]
pub enum Event {
    DeviceDiscovered { uuid: Uuid, rssi: i16 },
    NodeAdded(Result<Node, eyre::Error>),
}

#[derive(Debug)]
pub struct Node {
    uuid: Uuid,
    unicast_addr: MeshAddr,
}
