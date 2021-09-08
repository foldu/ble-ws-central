use super::{
    proto::MeshAddr,
    types::{AppIndex, BluezToken, NetIndex, ScanOptions, SendOptions},
};
use std::collections::HashMap;
use zbus::dbus_proxy;
use zvariant::{ObjectPath, OwnedValue, Value};

#[dbus_proxy(
    default_service = "org.bluez.mesh",
    default_path = "/org/bluez/mesh",
    interface = "org.bluez.mesh.Network1"
)]
trait Network1 {
    /// Attach method
    fn attach(
        &self,
        app: &ObjectPath<'_>,
        token: BluezToken,
    ) -> zbus::Result<(
        zvariant::OwnedObjectPath,
        Vec<(u8, Vec<(u16, HashMap<String, OwnedValue>)>)>,
    )>;

    /// Cancel method
    fn cancel(&self) -> zbus::Result<()>;

    /// CreateNetwork method
    fn create_network(&self, app: &ObjectPath<'_>, uuid: &[u8]) -> zbus::Result<()>;

    /// Import method
    fn import(
        &self,
        app: &ObjectPath<'_>,
        uuid: &[u8],
        dev_key: &[u8],
        net_key: &[u8],
        net_index: NetIndex,
        flags: HashMap<&str, Value<'_>>,
        iv_index: u32,
        unicast: u16,
    ) -> zbus::Result<()>;

    /// Join method
    fn join(&self, app: &ObjectPath<'_>, uuid: &[u8]) -> zbus::Result<()>;

    /// Leave method
    fn leave(&self, token: BluezToken) -> zbus::Result<()>;
}

#[dbus_proxy(
    default_service = "org.bluez.mesh",
    default_path = "/org/bluez/mesh",
    interface = "org.bluez.mesh.Node1"
)]
trait Node1 {
    fn send(
        &self,
        element_path: &ObjectPath<'_>,
        destination: MeshAddr,
        key_index: AppIndex,
        options: &SendOptions,
        data: Vec<u8>,
    ) -> Result<(), zbus::Error>;

    fn dev_key_send(
        &self,
        element_path: &ObjectPath<'_>,
        destination: MeshAddr,
        remote: bool,
        net_index: NetIndex,
        options: &SendOptions,
        data: Vec<u8>,
    ) -> Result<(), zbus::Error>;

    fn add_net_key(
        &self,
        element_path: &ObjectPath<'_>,
        destination: MeshAddr,
        app_index: AppIndex,
        net_index: NetIndex,
        update: bool,
    ) -> Result<(), zbus::Error>;

    fn add_app_key(
        &self,
        element_path: &ObjectPath<'_>,
        destination: MeshAddr,
        app_index: AppIndex,
        net_index: NetIndex,
        update: bool,
    ) -> Result<(), zbus::Error>;

    #[dbus_proxy(property)]
    fn addresses(&self) -> Result<Vec<MeshAddr>, zbus::Error>;
}

#[dbus_proxy(
    default_service = "org.bluez.mesh",
    default_path = "/org/bluez/mesh",
    interface = "org.bluez.mesh.Management1"
)]
trait Management1 {
    fn unprovisioned_scan(&self, options: &ScanOptions) -> Result<(), zbus::Error>;

    fn unprovisioned_scan_cancel(&self) -> Result<(), zbus::Error>;

    fn add_node(
        &self,
        uuid: &[u8],
        options: HashMap<String, zvariant::Value<'_>>,
    ) -> Result<(), zbus::Error>;

    fn create_app_key(&self, net_index: NetIndex, app_index: AppIndex) -> Result<(), zbus::Error>;

    fn import_app_key(
        &self,
        net_index: NetIndex,
        app_index: AppIndex,
        app_key: &[u8],
    ) -> Result<(), zbus::Error>;

    fn create_subnet(&self, net_index: NetIndex) -> Result<(), zbus::Error>;
}
