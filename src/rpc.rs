use crate::{db::IdDbEntry, sensor::SensorState};
use ble_ws_api::{
    data::Timestamp,
    proto::{
        self, ble_weatherstation_service_server::BleWeatherstationService, BackupRequest,
        BackupResponse, ChangeLabelRequest, ChangeLabelResponse, OverviewRequest, OverviewResponse,
        OverviewResponseField, SensorDataRequest, SensorDataResponse, SubscribeToChangesRequest,
    },
};
use futures_util::stream::TryStreamExt;
use std::{num::NonZeroUsize, sync::Arc};
use tokio_stream::wrappers::{BroadcastStream, ReceiverStream};
use tonic::{Request, Response, Status};
use uuid::Uuid;

impl From<crate::db::Error> for Status {
    fn from(_: crate::db::Error) -> Self {
        Status::internal("Internal database error")
    }
}

#[derive(derive_more::Deref, derive_more::Constructor)]
pub(crate) struct Server(Arc<crate::Context>);

type GenericStream<T> =
    Box<dyn futures_util::Stream<Item = Result<T, Status>> + Send + Sync + Unpin>;

const CHUNKSIZE: NonZeroUsize = nonzero_ext::nonzero!(8192usize);

#[tonic::async_trait]
impl BleWeatherstationService for Server {
    async fn get_sensor_data(
        &self,
        request: Request<SensorDataRequest>,
    ) -> Result<Response<SensorDataResponse>, Status> {
        let req = request.into_inner();
        let range = if req.start > req.end {
            return Err(Status::invalid_argument("Invalid time range"));
        } else {
            Timestamp::from(req.start)..Timestamp::from(req.end)
        };
        let txn = self.db.read_txn()?;
        match self
            .db
            .get_log(&txn, Uuid::from(req.id.unwrap()), range, Default::default())?
        {
            None => Err(Status::not_found("Sensor never existed")),
            Some(log) => Ok(Response::new(log)),
        }
    }

    async fn overview(
        &self,
        request: Request<OverviewRequest>,
    ) -> Result<Response<OverviewResponse>, Status> {
        // FIXME:
        let _ = request;
        let sensors = self.read_sensors().await;
        let mut ret = Vec::with_capacity(sensors.len());
        let txn = self.db.read_txn()?;
        for (id, sensor) in sensors.iter() {
            let label = self
                .db
                .get_addr(&txn, *id)?
                .and_then(|entry| entry.label)
                .map(|label| ble_ws_api::proto::Label { name: label });
            let overview = ble_ws_api::proto::SensorOverview {
                label,
                values: match sensor.state {
                    SensorState::Connected(values) => Some(ble_ws_api::proto::CurrentValues {
                        temperature: i32::from(values.temperature.as_i16()),
                        pressure: values.pressure.as_u32(),
                        humidity: u32::from(values.humidity.as_u16()),
                    }),

                    SensorState::Unconnected => None,
                },
            };

            ret.push(OverviewResponseField {
                id: Some(proto::Uuid::from(*id)),
                overview: Some(overview),
            });
        }

        Ok(Response::new(OverviewResponse { overview: ret }))
    }

    type SubscribeToChangesStream = Box<
        dyn futures_util::Stream<Item = Result<OverviewResponse, Status>> + Send + Sync + Unpin,
    >;
    type BackupStream = GenericStream<BackupResponse>;

    async fn subscribe_to_changes(
        &self,
        _request: Request<SubscribeToChangesRequest>,
    ) -> Result<Response<Self::SubscribeToChangesStream>, Status> {
        let rx = Box::new(
            BroadcastStream::new(self.tx.subscribe()).map_err(|_| Status::internal("tx dropped")),
        );
        Ok(Response::new(rx))
    }

    async fn change_label(
        &self,
        request: Request<ChangeLabelRequest>,
    ) -> Result<Response<ChangeLabelResponse>, Status> {
        let req = request.into_inner();
        let id = get_id(req.id)?;
        {
            let txn = self.0.db.read_txn()?;
            if self.0.db.get_addr(&txn, id)?.is_none() {
                return Err(Status::not_found("Sensor not found"));
            }
        }

        let new_label = req.label.map(|label| label.name);
        {
            let mut txn = self.0.db.write_txn()?;
            self.0.db.put_addr(
                &mut txn,
                id,
                &IdDbEntry {
                    label: new_label.clone(),
                },
            )?;
            txn.commit().map_err(|e| crate::db::Error::from(e))?;
        }

        tracing::info!("Changed label of {} to {:?}", id, new_label);

        self.0
            .modify_sensors(|sensors| match sensors.get_mut(&id) {
                Some(entry) => {
                    entry.label = new_label;
                }
                None => {
                    tracing::error!("Detected race in `change_label` handler");
                }
            })
            .await;

        Ok(Response::new(ChangeLabelResponse {}))
    }

    async fn backup(
        &self,
        _req: Request<BackupRequest>,
    ) -> Result<Response<Self::BackupStream>, Status> {
        let ctx = self.0.clone();
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<BackupResponse, ()>>(1);
        tokio::task::spawn_blocking(move || {
            // FIXME: timeout after some time if request takes too long
            // and release db lock
            let txn = ctx.db.read_txn().unwrap();
            for uuid in ctx.db.known_addrs(&txn).unwrap() {
                if let Ok(uuid) = uuid {
                    let mut start = Timestamp::from(0);
                    loop {
                        let resp = ctx
                            .db
                            .get_log(
                                &txn,
                                uuid,
                                start..Timestamp::from(u32::MAX),
                                Some(CHUNKSIZE),
                            )
                            .unwrap();
                        let resp = match resp {
                            Some(resp) => resp,
                            _ => break,
                        };
                        if let Some(last) = resp.time.last() {
                            start = Timestamp::from(
                                u32::from(*last).checked_add(1).expect("Timestamp overflow"),
                            );
                            if let Err(_) = tx.blocking_send(Ok(BackupResponse {
                                id: Some(ble_ws_api::proto::Uuid::from(uuid)),
                                chunk: Some(resp),
                            })) {
                                return;
                            }
                        } else {
                            break;
                        }
                    }
                }
            }
        });
        Ok(Response::new(Box::new(
            ReceiverStream::new(rx).map_err(|_| Status::internal("tx dropped")),
        )))
    }
}

fn get_id(a: Option<proto::Uuid>) -> Result<Uuid, Status> {
    a.ok_or_else(|| Status::new(tonic::Code::InvalidArgument, "Missing required id"))
        .map(Uuid::from)
}
