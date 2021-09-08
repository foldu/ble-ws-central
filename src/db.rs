use crate::sensor::{RawSensorValues, SensorValues};
use ble_ws_api::data::Timestamp;
use heed::{
    byteorder::BigEndian,
    types::{integer::U32, OwnedType, SerdeBincode},
    RoTxn,
};
use std::{
    collections::BTreeMap,
    fs,
    ops::Range,
    path::{Path, PathBuf},
    sync::{RwLock, RwLockReadGuard},
};
use uuid::Uuid;

type LEU32 = U32<BigEndian>;

type IdDb = BTreeMap<DbUuid, heed::Database<OwnedType<LEU32>, OwnedType<RawSensorValues>>>;

#[derive(
    bytemuck::Pod,
    bytemuck::Zeroable,
    Copy,
    Clone,
    Eq,
    PartialEq,
    Ord,
    PartialOrd,
    derive_more::Display,
)]
#[repr(transparent)]
struct DbUuid(u128);

impl From<Uuid> for DbUuid {
    fn from(other: Uuid) -> Self {
        Self(other.as_u128())
    }
}

impl DbUuid {
    fn as_uuid(self) -> Uuid {
        Uuid::from_u128(self.0)
    }
}

pub(crate) struct Db {
    env: heed::Env,
    id_db: heed::Database<OwnedType<DbUuid>, SerdeBincode<IdDbEntry>>,
    sensor_log: RwLock<IdDb>,
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
pub(crate) struct IdDbEntry {
    pub(crate) label: Option<String>,
}

pub(crate) struct LogTransaction<'a> {
    sensor_values: RwLockReadGuard<'a, IdDb>,
    txn: heed::RwTxn<'a, 'a>,
}

impl<'a> LogTransaction<'a> {
    pub(crate) fn log(&mut self, id: Uuid, values: SensorValues) -> Result<(), heed::Error> {
        let id = DbUuid::from(id);
        if let Some(db) = self.sensor_values.get(&id) {
            db.append(
                &mut self.txn,
                &LEU32::new(values.timestamp.as_u32()),
                &values.into(),
            )?;
        }
        Ok(())
    }

    pub(crate) fn commit(self) -> Result<(), Error> {
        self.txn.commit().map_err(heed_err)
    }
}

impl Db {
    pub fn open(db_path: impl AsRef<Path>) -> Result<Self, Error> {
        let db_path = db_path.as_ref();
        fs::create_dir_all(&db_path).map_err(|source| Error::Create {
            path: db_path.to_owned(),
            source,
        })?;

        let env = heed::EnvOpenOptions::new().max_dbs(200).open(db_path)?;
        let addr_db = env.create_database(Some("addr"))?;
        let ret = Self {
            env,
            id_db: addr_db,
            sensor_log: RwLock::new(BTreeMap::new()),
        };

        let known_addrs = {
            let txn = ret.read_txn()?;
            let it = ret.known_addrs(&txn)?;
            it.collect::<Result<Vec<_>, _>>()?
        };

        {
            let mut sensor_log = ret.sensor_log.write().unwrap();
            for id in known_addrs {
                let id = DbUuid::from(id);
                sensor_log.insert(id, ret.env.create_database(Some(&id.to_string()))?);
            }
        }

        Ok(ret)
    }

    pub fn read_txn(&self) -> Result<heed::RoTxn, Error> {
        self.env.read_txn().map_err(heed_err)
    }

    pub fn write_txn(&self) -> Result<heed::RwTxn, Error> {
        self.env.write_txn().map_err(heed_err)
    }

    pub fn log_txn(&self) -> Result<LogTransaction, Error> {
        Ok(LogTransaction {
            sensor_values: self.sensor_log.read().unwrap(),
            txn: self.write_txn()?,
        })
    }

    pub fn get_addr<'txn, T>(
        &self,
        txn: &'txn RoTxn<'_, T>,
        id: Uuid,
    ) -> Result<Option<IdDbEntry>, Error> {
        self.id_db.get(txn, &DbUuid::from(id)).map_err(heed_err)
    }

    pub fn put_addr(
        &self,
        txn: &mut heed::RwTxn<'_, '_>,
        id: Uuid,
        data: &IdDbEntry,
    ) -> Result<(), Error> {
        let id = DbUuid::from(id);
        self.id_db.put(txn, &id, data).map_err(heed_err)
    }

    pub fn known_addrs<'txn, T>(
        &self,
        txn: &'txn RoTxn<'_, T>,
    ) -> Result<impl Iterator<Item = Result<Uuid, Error>> + 'txn, Error> {
        self.id_db
            .iter(txn)
            .map(|it| it.map(|res| res.map(|(id, _)| id.as_uuid()).map_err(heed_err)))
            .map_err(heed_err)
    }

    pub fn delete_addr(&self, txn: &mut heed::RwTxn<'_, '_>, id: Uuid) -> Result<bool, Error> {
        let id = DbUuid::from(id);
        self.id_db.delete(txn, &id).map_err(heed_err)
    }

    pub fn get_log<T>(
        &self,
        txn: &RoTxn<'_, T>,
        id: Uuid,
        range: Range<Timestamp>,
    ) -> Result<Option<ble_ws_api::proto::SensorDataResponse>, Error> {
        let sensor_log = self.sensor_log.read().unwrap();
        let id = DbUuid::from(id);
        let db = match sensor_log.get(&id) {
            Some(db) => db,
            _ => return Ok(None),
        };

        let range = LEU32::new(range.start.as_u32())..LEU32::new(range.end.as_u32());

        let mut data = ble_ws_api::proto::SensorDataResponse::default();
        for val in db.range(txn, &range)? {
            let (time, values) = val?;
            data.time.push(time.get());
            data.temperature.push(values.temperature as i32);
            data.pressure.push(values.pressure);
            data.humidity.push(values.humidity as u32);
        }

        Ok(Some(data))
    }
}

#[derive(thiserror::Error, Debug)]
pub(crate) enum Error {
    #[error("Can't create database dir in {}", path.display())]
    Create {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("Error in database backend")]
    Heed(#[source] Box<dyn std::error::Error + Send + Sync>),
}

fn heed_err(e: heed::Error) -> Error {
    Error::Heed(format!("{}", e).into())
}

impl From<heed::Error> for Error {
    fn from(e: heed::Error) -> Self {
        heed_err(e)
    }
}
