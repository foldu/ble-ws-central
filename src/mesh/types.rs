use std::fmt::Display;

#[derive(zvariant::derive::SerializeDict, zvariant::derive::TypeDict, Default)]
pub struct SendOptions {
    #[zvariant(rename = "ForceSegmented")]
    pub force_segmented: bool,
}

#[derive(zvariant::derive::SerializeDict, zvariant::derive::TypeDict, Default)]
pub struct ScanOptions {
    #[zvariant(rename = "Seconds")]
    pub seconds: u16,
}

#[derive(serde::Serialize, serde::Deserialize, zvariant::derive::Type, Clone, Copy)]
pub struct BluezToken(u64);

// NOTE: net_index == subnet_index
#[derive(
    serde::Serialize, serde::Deserialize, zvariant::derive::Type, Clone, Copy, Debug, PartialEq, Eq,
)]
pub struct NetIndex(pub u16);

impl Display for NetIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Display::fmt(&self.0, f)
    }
}

#[derive(
    serde::Serialize, serde::Deserialize, zvariant::derive::Type, Clone, Copy, Debug, PartialEq, Eq,
)]
pub struct AppIndex(pub u16);

impl Display for AppIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        Display::fmt(&self.0, f)
    }
}
