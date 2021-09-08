use crate::sensor::{SensorState, SensorValues};
use ble_ws_api::data::{Celsius, Pascal, RelativeHumidity, Timestamp};
use futures_util::stream::Stream;
use rand::prelude::*;
use std::{collections::BTreeMap, convert::TryFrom, ops::Range, time::Duration};
use tokio::sync::mpsc;
use uuid::Uuid;

struct FluctuatingSensor {
    humidity: u16,
    temperature: i16,
    pressure: u32,
}

impl Default for FluctuatingSensor {
    fn default() -> Self {
        Self {
            humidity: 50_00,
            pressure: 1_000_000_0,
            temperature: 20_00,
        }
    }
}

fn clamp<T>(n: T, lo: T, hi: T) -> T
where
    T: Ord + Copy,
{
    if n < lo {
        lo
    } else if n > hi {
        hi
    } else {
        n
    }
}

impl FluctuatingSensor {
    fn advance(&mut self, timestamp: Timestamp) -> SensorValues {
        let mut rng = rand::thread_rng();
        self.temperature = clamp(self.temperature + rng.gen_range(-1_00..1_00), 0_00, 30_00);
        self.pressure = clamp(
            // TODO: better range for pressure
            if rng.gen::<bool>() {
                self.pressure + rng.gen_range(100..1000)
            } else {
                self.pressure - rng.gen_range(100..1000)
            },
            90_000_0,
            110_000_0,
        );
        self.humidity = clamp(
            if rng.gen::<bool>() {
                self.humidity + rng.gen_range(0_20..0_90)
            } else {
                self.humidity - rng.gen_range(0_20..0_90)
            },
            20_00,
            90_00,
        );
        SensorValues {
            timestamp,
            humidity: RelativeHumidity::try_from(self.humidity).unwrap(),
            pressure: Pascal::from(self.pressure),
            temperature: Celsius::try_from(self.temperature).unwrap(),
        }
    }
}

pub(crate) fn dummy_sensors(
    addrs: Range<u128>,
) -> impl Stream<Item = BTreeMap<uuid::Uuid, SensorState>> + Sync + Send {
    let (tx, rx) = mpsc::channel(1);
    tokio::task::spawn(async move {
        let mut sensors = addrs
            .into_iter()
            .map(|id| (Uuid::from_u128(id), FluctuatingSensor::default()))
            .collect::<Vec<_>>();
        let mut map = BTreeMap::new();

        loop {
            map.clear();
            let now = Timestamp::now();
            for (addr, sensor) in &mut sensors {
                map.insert(*addr, SensorState::Connected(sensor.advance(now)));
            }

            if let Err(_) = tx.send(map.clone()).await {
                break;
            }
            tokio::time::sleep(Duration::from_secs(30)).await;
        }
    });

    tokio_stream::wrappers::ReceiverStream::new(rx)
}
