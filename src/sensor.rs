use ble_ws_api::data::{Celsius, Pascal, RelativeHumidity, Timestamp};
use std::fmt::{self, Display};

#[derive(Copy, Clone, Debug, serde::Serialize)]
pub(crate) struct SensorValues {
    pub timestamp: Timestamp,
    pub temperature: Celsius,
    pub pressure: Pascal,
    pub humidity: RelativeHumidity,
}

impl Display for SensorValues {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Humidity: {}, Temperature: {}, pressure: {}",
            self.humidity, self.temperature, self.pressure
        )
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum SensorState {
    Connected(SensorValues),
    Unconnected,
}

#[repr(C)]
#[derive(Copy, Clone, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct RawSensorValues {
    pub temperature: i16,
    pub humidity: u16,
    pub pressure: u32,
}

impl From<SensorValues> for RawSensorValues {
    fn from(values: SensorValues) -> Self {
        Self {
            temperature: values.temperature.as_i16(),
            pressure: values.pressure.as_u32(),
            humidity: values.humidity.as_u16(),
        }
    }
}
