use crate::{db, sensor::SensorState};
use std::{collections::BTreeMap, fmt::Write, sync::Arc, time::Duration};
use tokio_stream::{Stream, StreamExt};
use uuid::Uuid;

pub(crate) async fn mqtt_publish(
    ctx: Arc<super::Context>,
    mut cxn: tokio_mqtt::Connection,
) -> Result<(), tokio_mqtt::Error> {
    let mut topic_buf = String::new();
    let mut interval = tokio::time::interval(Duration::from_secs(60));
    let mut json_buf = Vec::new();
    loop {
        interval.tick().await;
        let sensors = ctx.read_sensors().await;
        for (addr, sensor) in &*sensors {
            if let SensorState::Connected(values) = &sensor.state {
                topic_buf.clear();
                write!(topic_buf, "sensors/weatherstation/{}", addr).unwrap();
                serde_json::to_writer(std::io::Cursor::new(&mut json_buf), &values).unwrap();
                // TODO: figure out what happens when mqtt server dies
                if let Err(e) = cxn
                    .publish(
                        tokio_mqtt::TopicName::new(topic_buf.clone()).unwrap(),
                        json_buf.clone(),
                    )
                    .await
                {
                    tracing::error!("Failed publishing to mqtt server: {}", e);
                }
            }
        }
    }
}

pub(crate) async fn update(
    ctx: Arc<super::Context>,
    mut updates: impl Stream<Item = BTreeMap<Uuid, SensorState>> + Unpin,
) -> Result<(), db::Error> {
    while let Some(update) = updates.next().await {
        let mut new_sensors = Vec::new();
        {
            let txn = ctx.db.read_txn()?;
            for &addr in update.keys() {
                if ctx.db.get_addr(&txn, addr)?.is_none() {
                    new_sensors.push(addr);
                    tracing::info!("Memorized new sensor {}", addr);
                }
            }
        }
        if !new_sensors.is_empty() {
            let mut txn = ctx.db.write_txn()?;
            for addr in new_sensors {
                ctx.db.put_addr(&mut txn, addr, &db::IdDbEntry::default())?;
            }
            txn.commit()?;
        }

        {
            let mut txn = ctx.db.log_txn()?;
            for (addr, state) in &update {
                if let SensorState::Connected(values) = state {
                    txn.log(*addr, *values)?;
                }
            }
            txn.commit()?;
        }

        ctx.modify_sensors(move |sensors| {
            for (addr, state) in update {
                match sensors.get_mut(&addr) {
                    Some(sensor) => {
                        sensor.state = state;
                    }
                    None => {
                        sensors.insert(addr, crate::Sensor { label: None, state });
                    }
                }
            }
        })
        .await;
    }
    Ok(())
}
