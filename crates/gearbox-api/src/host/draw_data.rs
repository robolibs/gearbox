use std::sync::{Mutex, PoisonError};

use agentio::{Agent, Registered};
use peerbus::ReqServer;

use crate::topics;
use crate::wire::{Env, draw_data::*};

pub const DATA_POLL_PER_ROUTE: usize = 16;

pub struct DataLane {
    epoch: u64,
    servers: [Mutex<Registered<ReqServer<Env, Env>>>; 4],
    wire_only: DataCapabilities,
}

impl DataLane {
    pub fn new(agent: &Agent) -> agentio::Result<Self> {
        let epoch = loop {
            let value =
                getrandom::u64().map_err(|e| agentio::Error::Configuration(e.to_string()))?;
            if value != 0 {
                break value;
            }
        };
        Ok(Self {
            epoch,
            servers: [
                Mutex::new(agent.req_server(topics::DRAW_DATA_CAPABILITIES)?),
                Mutex::new(agent.req_server(topics::DRAW_DATA_STATUS)?),
                Mutex::new(agent.req_server(topics::DRAW_DATA_LIST)?),
                Mutex::new(agent.req_server(topics::DRAW_DATA)?),
            ],
            wire_only: DataCapabilities::wire_only(epoch),
        })
    }

    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn serve_unavailable(&self) -> agentio::Result<usize> {
        self.serve(&self.wire_only, |_, _| {
            DataReply::error(
                self.epoch,
                data_code::UNSUPPORTED,
                "spatial dataset storage is unavailable",
            )
        })
    }

    /// Polls a bounded batch per route; handlers run outside transport locks.
    pub fn serve(
        &self,
        capabilities: &DataCapabilities,
        mut handle: impl FnMut(DataRequest, DataOperation) -> DataReply,
    ) -> agentio::Result<usize> {
        self.serve_requests(capabilities, |request| match request.operation() {
            Ok(operation) => handle(request, operation),
            Err(error) => DataReply::error(self.epoch, data_code::INVALID, &error),
        })
    }

    pub fn serve_store(
        &self,
        store: &mut crate::draw_data::StreamingStore,
        now_ms: u64,
    ) -> agentio::Result<usize> {
        store
            .maintain(now_ms)
            .map_err(|error| agentio::Error::Configuration(error.to_string()))?;
        self.serve_requests(&store.capabilities(), |request| {
            store.handle(&request, now_ms)
        })
    }

    fn serve_requests(
        &self,
        capabilities: &DataCapabilities,
        mut handle: impl FnMut(DataRequest) -> DataReply,
    ) -> agentio::Result<usize> {
        if capabilities.host_epoch != self.epoch {
            return Err(agentio::Error::Configuration(
                "capability host epoch mismatch".into(),
            ));
        }
        let metadata = capabilities
            .to_json()
            .map_err(agentio::Error::Configuration)?;
        let mut served = 0;
        for (route, server) in self.servers.iter().enumerate() {
            for _ in 0..DATA_POLL_PER_ROUTE {
                let pending = server
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .take_message()?;
                let Some(pending) = pending else {
                    break;
                };
                let (sample, token) = pending.into_parts();
                let reply = match DataRequest::unpack_bounded(
                    sample.header().type_hash,
                    sample.payload(),
                ) {
                    Err(error) => DataReply::error(self.epoch, data_code::INVALID, &error),
                    Ok(request) => self.dispatch(route, request, &metadata, &mut handle),
                };
                let env = if reply.host_epoch == self.epoch {
                    reply.pack_checked().ok()
                } else {
                    None
                }
                .unwrap_or_else(|| {
                    DataReply::error(
                        self.epoch,
                        data_code::INTERNAL,
                        "invalid dataset handler reply",
                    )
                    .pack_checked()
                    .expect("bounded internal reply")
                });
                server
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .respond_pending(token, &env)?;
                served += 1;
            }
        }
        Ok(served)
    }

    fn dispatch(
        &self,
        route: usize,
        request: DataRequest,
        capabilities: &[u8],
        handle: &mut impl FnMut(DataRequest) -> DataReply,
    ) -> DataReply {
        let expected = match request.operation {
            data_op::CAPABILITIES => 0,
            data_op::STATUS | data_op::FIELD_QUERY => 1,
            data_op::LIST => 2,
            _ => 3,
        };
        if route != expected {
            return DataReply::error(
                self.epoch,
                data_code::INVALID,
                "operation sent to the wrong data topic",
            );
        }
        if request.operation == data_op::CAPABILITIES {
            if let Err(error) = request.operation() {
                return DataReply::error(self.epoch, data_code::INVALID, &error);
            }
            return DataReply {
                metadata: capabilities.to_vec(),
                ..DataReply::new(self.epoch)
            };
        }
        if request.host_epoch != self.epoch {
            return DataReply::error(
                self.epoch,
                data_code::STALE_EPOCH,
                "host restarted; negotiate a new writer",
            );
        }
        handle(request)
    }
}
