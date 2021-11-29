use super::{
    proto::{MeshAddr, Opcode},
    types::{AppIndex, NetIndex},
};
use ble_ws_api::data::Timestamp;
use slotmap::{DenseSlotMap, SecondaryMap};
use std::sync::Arc;
use tokio::sync::{mpsc, oneshot};

slotmap::new_key_type! {
    struct MessageKey;
    struct FilterKey;
}

#[derive(Debug)]
pub enum MessageMeta {
    Device {
        source_addr: MeshAddr,
        net_index: NetIndex,
    },
    Normal {
        source_addr: MeshAddr,
        app_index: AppIndex,
    },
}

#[derive(Clone)]
pub struct MessageQueue(Arc<MessageQueueInner>);

struct MessageQueueInner {
    subscription_tx: mpsc::Sender<(GenericFilter, Subscriber)>,
    msg_tx: mpsc::Sender<GenericMessage>,
}

#[derive(PartialEq, Eq, Copy, Clone, Debug)]
pub enum MessageKind {
    Device,
    Normal,
}

#[derive(Default)]
struct State {
    messages: Vec<GenericMessage>,
    filters: DenseSlotMap<FilterKey, GenericFilter>,
    subscribers: SecondaryMap<FilterKey, Subscriber>,
}

#[derive(Debug)]
pub struct GenericMessage {
    pub source_addr: MeshAddr,
    pub received: Timestamp,
    pub kind: MessageKind,
    pub content: super::proto::Message,
    pub index: u16,
}

impl GenericMessage {
    fn new(msg: super::proto::Message, meta: MessageMeta) -> Self {
        let received = Timestamp::now();
        match meta {
            MessageMeta::Device {
                source_addr,
                net_index,
            } => Self {
                source_addr,
                index: net_index.0,
                received,
                kind: MessageKind::Device,
                content: msg,
            },
            MessageMeta::Normal {
                source_addr,
                app_index,
            } => Self {
                source_addr,
                index: app_index.0,
                received,
                kind: MessageKind::Normal,
                content: msg,
            },
        }
    }
}

impl GenericFilter {
    fn is_match(&self, message: &GenericMessage) -> bool {
        // FIXME: ugly
        if message.kind == self.kind {
            match self.source_addr {
                Some(source_addr) if source_addr != message.source_addr => {
                    return false;
                }
                _ => (),
            }

            match self.opcode {
                Some(opcode) if opcode != message.content.opcode => {
                    return false;
                }
                _ => (),
            }

            match self.index {
                Some(index) if index != message.index => {
                    return false;
                }
                _ => (),
            }

            true
        } else {
            false
        }
    }
}

fn iter_filter<'a>(
    filter: &'a GenericFilter,
    messages: &'a [GenericMessage],
) -> impl Iterator<Item = usize> + 'a {
    let mut it = messages.iter().enumerate();
    std::iter::from_fn(move || loop {
        loop {
            let (i, msg) = it.next()?;
            if filter.is_match(&msg) {
                return Some(i);
            }
        }
    })
}

impl State {
    fn try_pull_one(
        &mut self,
        tx_cell: &mut Option<oneshot::Sender<GenericMessage>>,
        filter: &GenericFilter,
    ) -> bool {
        let mut it = iter_filter(filter, &self.messages);
        if let Some(index) = it.next() {
            drop(it);
            let msg = self.messages.remove(index);
            if let Err(msg) = tx_cell.take().unwrap().send(msg) {
                self.messages.push(msg);
                return false;
            }
            return true;
        }

        false
    }

    async fn try_pull_many(
        &mut self,
        tx: &mut mpsc::Sender<GenericMessage>,
        filter: &GenericFilter,
    ) {
        // NOTE: borrows self.messages, so need to buffer
        let indexes = iter_filter(filter, &self.messages).collect::<Vec<_>>();
        for index in indexes {
            let msg = self.messages.remove(index);
            if let Err(_) = tx.send(msg).await {
                todo!("Retry logic in try_pull_many")
            }
        }
    }

    fn add_subscriber(&mut self, filter: GenericFilter, subscriber: Subscriber) {
        let key = self.filters.insert(filter);
        self.subscribers.insert(key, subscriber);
    }

    fn match_filter(&self, msg: &GenericMessage) -> Option<FilterKey> {
        self.filters
            .iter()
            .find(|(_, filter)| filter.is_match(msg))
            .map(|(i, _)| i)
    }
}

impl MessageQueue {
    pub fn new(handle: &tokio::runtime::Handle) -> Self {
        let (msg_tx, mut msg_rx) = mpsc::channel(1);
        let (subscription_tx, mut subscribe_rx) = mpsc::channel::<(GenericFilter, Subscriber)>(1);
        let mut state = State::default();

        handle.spawn(async move {
            loop {
                tokio::select! {
                    Some((filter, chan)) = subscribe_rx.recv() => {
                        let mut chan = chan;
                        let do_add = match chan {
                            Subscriber::Oneshot(ref mut tx) => {
                                if state.try_pull_one(tx, &filter) {
                                    false
                                } else {
                                    true
                                }
                            }
                            Subscriber::Stream(ref mut tx) => {
                                state.try_pull_many(tx, &filter).await;
                                true
                            }
                        };
                        if do_add {
                            state.add_subscriber(filter, chan);
                        }
                    }
                    Some(msg) = msg_rx.recv() => {
                        if let Some(index) = state.match_filter(&msg) {
                            let rm = match state.subscribers.get_mut(index).expect("Inconsistent state") {
                                Subscriber::Oneshot(tx) => {
                                    if let Err(_) = tx.take().expect("Reusing oneshot tx").send(msg) {
                                        todo!("Closed tx, need to implement retry logic looping over filters");
                                    } 
                                    true
                                }
                                Subscriber::Stream(tx) => {
                                    if let Err(_) = tx.send(msg).await {
                                        todo!("Closed tx, need to implement retry logic looping over filters");
                                    }
                                    false
                                }
                            };
                            if rm {
                                state.subscribers.remove(index);
                                state.filters.remove(index);
                            }
                        } else {
                            state.messages.push(msg);
                        }
                    }
                }
            }
        });

        Self(Arc::new(MessageQueueInner {
            subscription_tx,
            msg_tx,
        }))
    }
}

#[derive(Debug)]
enum Subscriber {
    Oneshot(Option<oneshot::Sender<GenericMessage>>),
    Stream(mpsc::Sender<GenericMessage>),
}

#[derive(Copy, Clone, Debug)]
pub struct GenericFilter {
    source_addr: Option<MeshAddr>,
    index: Option<u16>,
    opcode: Option<Opcode>,
    kind: MessageKind,
}

#[derive(Default, Clone, Copy)]
pub struct DevFilter {
    source_addr: Option<MeshAddr>,
    net_index: Option<NetIndex>,
    opcode: Option<Opcode>,
}

impl DevFilter {
    pub fn source_addr(mut self, source_addr: MeshAddr) -> Self {
        self.source_addr = Some(source_addr);
        self
    }

    pub fn net_index(mut self, net_index: NetIndex) -> Self {
        self.net_index = Some(net_index);
        self
    }

    pub fn opcode(mut self, opcode: Opcode) -> Self {
        self.opcode = Some(opcode);
        self
    }
}

impl From<DevFilter> for GenericFilter {
    fn from(f: DevFilter) -> Self {
        Self {
            kind: MessageKind::Device,
            opcode: f.opcode,
            index: f.net_index.map(|i| i.0),
            source_addr: f.source_addr,
        }
    }
}

#[derive(Default, Clone, Copy)]
pub struct Filter {
    source_addr: Option<MeshAddr>,
    app_index: Option<AppIndex>,
    opcode: Option<Opcode>,
}

impl Filter {
    pub fn source_addr(mut self, source_addr: MeshAddr) -> Self {
        self.source_addr = Some(source_addr);
        self
    }

    pub fn app_index(mut self, app_index: AppIndex) -> Self {
        self.app_index = Some(app_index);
        self
    }

    pub fn opcode(mut self, opcode: Opcode) -> Self {
        self.opcode = Some(opcode);
        self
    }
}

impl From<Filter> for GenericFilter {
    fn from(f: Filter) -> Self {
        Self {
            kind: MessageKind::Normal,
            opcode: f.opcode,
            index: f.app_index.map(|i| i.0),
            source_addr: f.source_addr,
        }
    }
}

impl MessageQueue {
    pub fn push_msg(&self, msg: super::proto::Message, meta: MessageMeta) {
        let msg = GenericMessage::new(msg, meta);
        self.0.msg_tx.blocking_send(msg).unwrap();
    }

    pub async fn pull_msg<T, F>(&self, filter: F) -> Option<(MeshAddr, T)>
    where
        T: super::proto::ParsePacket,
        F: Into<GenericFilter>,
    {
        let (tx, rx) = oneshot::channel();
        let mut filter = filter.into();
        filter.opcode = Some(T::opcode());
        self.0
            .subscription_tx
            .send((filter, Subscriber::Oneshot(Some(tx))))
            .await
            .expect("Message queue crashed");
        let msg = rx.await.unwrap();
        let addr = msg.source_addr;
        let ret = T::parse_payload(&msg.content.payload)?;
        Some((addr, ret))
    }

    pub async fn subscribe<F>(&self, filter: F) -> mpsc::Receiver<GenericMessage>
    where
        F: Into<GenericFilter>,
    {
        let (tx, rx) = mpsc::channel(1);
        let filter = filter.into();
        self.0
            .subscription_tx
            .send((filter, Subscriber::Stream(tx)))
            .await
            .expect("Message queue crashed");
        rx
    }
}
