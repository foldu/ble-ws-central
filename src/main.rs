mod config;
mod db;
mod dummy;
mod mesh;
mod opt;
mod rpc;
mod sensor;
mod tasks;
mod util;

use crate::{dummy::dummy_sensors, opt::Opt};
use ble_ws_api::proto::{
    self, ble_weatherstation_service_server::BleWeatherstationServiceServer, OverviewResponse,
    OverviewResponseField,
};
use clap::Clap;
use config::Config;
use eyre::Context as _;
use futures_util::stream::{self, Stream};
use sensor::SensorState;
use std::{collections::BTreeMap, net::SocketAddr, sync::Arc};
use tokio::{
    signal::unix,
    sync::{broadcast::Sender, RwLock, RwLockReadGuard},
    task,
};
use unix::SignalKind;

fn main() -> Result<(), eyre::Error> {
    let args = Opt::parse();

    if std::env::var_os("RUST_LOG").is_none() {
        std::env::set_var("RUST_LOG", "info");
    }

    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    match args.executor {
        opt::Rt::MultiThread => {
            let mut builder = tokio::runtime::Builder::new_multi_thread();
            let rt = if let Some(n) = args.workers {
                builder.worker_threads(n.get())
            } else {
                &mut builder
            }
            .enable_all()
            .build()?;

            rt.block_on(run())
        }
        opt::Rt::CurrentThread => {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()?;
            rt.block_on(run())
        }
    }
}

type UpdateItem = BTreeMap<uuid::Uuid, SensorState>;
type UpdateSource = dyn Stream<Item = UpdateItem> + Unpin + Send;

async fn run() -> Result<(), eyre::Error> {
    let config = Config::from_env()?;
    let handle = tokio::runtime::Handle::current();

    let ctx = Context::create(&config).map(Arc::new)?;

    let mut sources: Vec<Box<UpdateSource>> = Vec::new();

    if config.enable_mesh {
        sources.push(mesh::listen(handle)?);
    }

    // sources.push(Box::new(bluetooth_update.into_stream()));
    if let Some(n) = config.demo {
        tracing::info!("Simulating {} dummy sensors", n);
        sources.push(Box::new(dummy_sensors(0..u128::from(n.get()))));
    }

    let update_task = task::spawn(tasks::update(ctx.clone(), stream::select_all(sources)));

    if let Some(ref options) = config.mqtt_options {
        let (cxn, _) =
            tokio_mqtt::Connection::connect(options, "ble-weatherstation-central", 60).await?;
        task::spawn(tasks::mqtt_publish(ctx.clone(), cxn));
    }

    let mut term = unix::signal(SignalKind::terminate()).unwrap();
    let mut int = unix::signal(SignalKind::interrupt()).unwrap();
    let shutdown = async move {
        // FIXME:
        let signal = async move {
            tokio::select! {
                _ = term.recv() => (),
                _ = int.recv() => ()
            }
        };

        tokio::select! {
            // TODO: unify crash error cases
            Err(e) = update_task => {
                tracing::error!("Update task failed: {}", e);
            }
            // _ = bluetooth_failed => {
            // }
            _ = signal => {
                // drop(stopped_tx);
            }
        }
    };

    let token = Arc::new(config.token);
    let service = BleWeatherstationServiceServer::with_interceptor(
        rpc::Server::new(ctx.clone()),
        move |req| match req.metadata().get("authorization") {
            Some(t) if *token == t => Ok(req),
            _ => Err(tonic::Status::unauthenticated("No valid auth token")),
        },
    );

    tonic::transport::Server::builder()
        .add_service(service)
        .serve_with_shutdown(SocketAddr::new(config.host, config.port), shutdown)
        .await?;

    Ok(())
}

impl Context {
    pub fn create(config: &Config) -> Result<Self, eyre::Error> {
        let db = db::Db::open(&config.db_path)
            .with_context(|| format!("Opening database in {}", config.db_path.display()))?;

        let mut sensors = BTreeMap::new();
        {
            let txn = db.read_txn()?;

            for id in db.known_addrs(&txn)? {
                let id = id?;
                let label = db.get_addr(&txn, id)?.and_then(|label| label.label);
                sensors.insert(
                    id,
                    Sensor {
                        label,
                        state: sensor::SensorState::Unconnected,
                    },
                );
            }
        }

        let (tx, _) = tokio::sync::broadcast::channel(1);
        Ok(Self {
            db,
            sensors: RwLock::new(sensors),
            tx,
        })
    }

    pub async fn read_sensors(&self) -> RwLockReadGuard<'_, Sensors> {
        self.sensors.read().await
    }

    pub async fn modify_sensors<F>(&self, f: F)
    where
        F: FnOnce(&mut Sensors),
    {
        let mut sensors = self.sensors.write().await;
        f(&mut *sensors);
        let _ = self.tx.send(create_overview_response(&sensors));
    }
}

fn create_overview_response(sensors: &Sensors) -> OverviewResponse {
    let mut overview = Vec::new();
    for (id, sensor) in sensors.iter() {
        let sensor_overview = ble_ws_api::proto::SensorOverview {
            label: sensor.label.as_ref().map(|name| ble_ws_api::proto::Label {
                name: name.to_owned(),
            }),
            values: match sensor.state {
                SensorState::Connected(values) => Some(ble_ws_api::proto::CurrentValues {
                    temperature: i32::from(values.temperature.as_i16()),
                    pressure: values.pressure.as_u32(),
                    humidity: u32::from(values.humidity.as_u16()),
                }),

                SensorState::Unconnected => None,
            },
        };

        overview.push(OverviewResponseField {
            id: Some(proto::Uuid::from(*id)),
            overview: Some(sensor_overview),
        });
    }

    OverviewResponse { overview }
}

pub(crate) struct Sensor {
    pub label: Option<String>,
    pub state: sensor::SensorState,
}

type Sensors = BTreeMap<uuid::Uuid, Sensor>;

pub(crate) struct Context {
    sensors: RwLock<Sensors>,
    pub(crate) db: db::Db,
    pub(crate) tx: Sender<OverviewResponse>,
}
