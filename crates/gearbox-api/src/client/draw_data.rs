use std::fmt;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use agentio::{ExchangeKind, ResolveRequest, ResolveResponse};
use peerbus::{EndpointId, ReqClient, wire_type_hash};

use super::Client;
use crate::topics;
use crate::wire::{Env, draw_data::*};

mod pipeline;
mod upload;
mod visibility;
pub use pipeline::{
    DataAdmission, DataCancelOutcome, DataPipelineConfig, DataPipelineError, DataPipelineStats,
    DataRequestCancellation, DataRequestTicket, DrawDataPipeline,
};
pub use upload::{DataUpload, DataUploadCleanup, DataUploadError, DataUploadPage, DrawDataWriter};
pub use visibility::WaitVisibleError;

pub const DEFAULT_DATA_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug)]
pub enum DrawDataError {
    Timeout,
    Transport(agentio::Error),
    Protocol(String),
    Unsupported(String),
    Rejected { code: u32, message: String },
}

impl fmt::Display for DrawDataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Timeout => write!(
                f,
                "data deadline elapsed; a delivered mutation may still complete"
            ),
            Self::Transport(error) => write!(f, "data transport: {error}"),
            Self::Protocol(message) => write!(f, "data protocol: {message}"),
            Self::Unsupported(message) => write!(f, "unsupported spatial data: {message}"),
            Self::Rejected { code, message } => {
                write!(f, "data request rejected ({code}): {message}")
            }
        }
    }
}

impl std::error::Error for DrawDataError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Transport(error) => Some(error),
            _ => None,
        }
    }
}

impl From<agentio::Error> for DrawDataError {
    fn from(error: agentio::Error) -> Self {
        Self::Transport(error)
    }
}

impl From<peerbus::Error> for DrawDataError {
    fn from(error: peerbus::Error) -> Self {
        match error {
            peerbus::Error::Timeout(_) => Self::Timeout,
            error => Self::Transport(error.into()),
        }
    }
}

/// A negotiated synchronous data connection to one host instance; each request opens
/// its own requester on the negotiated route.
pub struct DrawDataClient {
    capabilities: DataCapabilities,
    node: peerbus::Node,
    host: EndpointId,
}

impl Client {
    pub fn draw_data_capabilities(&self) -> Result<DataCapabilities, DrawDataError> {
        self.draw_data_capabilities_with_timeout(DEFAULT_DATA_TIMEOUT)
    }

    pub fn draw_data_capabilities_with_timeout(
        &self,
        timeout: Duration,
    ) -> Result<DataCapabilities, DrawDataError> {
        self.draw_data_capabilities_with_deadline(deadline_after(timeout)?)
    }

    pub fn draw_data_capabilities_with_deadline(
        &self,
        deadline: Instant,
    ) -> Result<DataCapabilities, DrawDataError> {
        capabilities(self, self.data_host()?, deadline)
    }

    pub fn draw_data(&self) -> Result<DrawDataClient, DrawDataError> {
        self.draw_data_with_timeout(DEFAULT_DATA_TIMEOUT)
    }

    /// Negotiate all data routes within one timeout; individual requests have their own deadlines.
    pub fn draw_data_with_timeout(
        &self,
        timeout: Duration,
    ) -> Result<DrawDataClient, DrawDataError> {
        self.draw_data_with_deadline(deadline_after(timeout)?)
    }

    pub fn draw_data_with_deadline(
        &self,
        deadline: Instant,
    ) -> Result<DrawDataClient, DrawDataError> {
        self.draw_data_at_with_deadline(self.data_host()?, deadline)
    }

    pub fn draw_data_at(&self, host: EndpointId) -> Result<DrawDataClient, DrawDataError> {
        self.draw_data_at_with_timeout(host, DEFAULT_DATA_TIMEOUT)
    }

    pub fn draw_data_at_with_timeout(
        &self,
        host: EndpointId,
        timeout: Duration,
    ) -> Result<DrawDataClient, DrawDataError> {
        self.draw_data_at_with_deadline(host, deadline_after(timeout)?)
    }

    pub fn draw_data_at_with_deadline(
        &self,
        host: EndpointId,
        deadline: Instant,
    ) -> Result<DrawDataClient, DrawDataError> {
        check_deadline(deadline)?;
        let capabilities = capabilities(self, host, deadline)?;
        for topic in [topics::DRAW_DATA, topics::DRAW_DATA_LIST, topics::DRAW_DATA_STATUS] {
            data_topic(self, host, topic, deadline)?;
        }
        check_deadline(deadline)?;
        Ok(DrawDataClient {
            capabilities,
            node: self.agent.node().clone(),
            host,
        })
    }

    fn data_host(&self) -> Result<EndpointId, DrawDataError> {
        self.host.ok_or_else(|| {
            DrawDataError::Protocol(
                "bare clients must use draw_data_at with an explicit host".into(),
            )
        })
    }
}

impl DrawDataClient {
    pub fn capabilities(&self) -> &DataCapabilities {
        &self.capabilities
    }

    /// Sends one checked request without retries or a background submission queue.
    pub fn request(&mut self, request: &DataRequest) -> Result<DataReply, DrawDataError> {
        self.request_with_timeout(request, DEFAULT_DATA_TIMEOUT)
    }

    /// Timeout leaves a delivered mutation's outcome unknown; this method never retries it.
    pub fn request_with_timeout(
        &mut self,
        request: &DataRequest,
        timeout: Duration,
    ) -> Result<DataReply, DrawDataError> {
        self.request_with_deadline(request, deadline_after(timeout)?)
    }

    pub fn request_with_deadline(
        &mut self,
        request: &DataRequest,
        deadline: Instant,
    ) -> Result<DataReply, DrawDataError> {
        check_deadline(deadline)?;
        request.operation().map_err(DrawDataError::Protocol)?;
        if request.host_epoch != self.capabilities.host_epoch {
            return Err(DrawDataError::Protocol(
                "request does not match the negotiated host epoch".into(),
            ));
        }
        let topic = match request.operation {
            data_op::LIST => topics::DRAW_DATA_LIST,
            data_op::STATUS | data_op::FIELD_QUERY => topics::DRAW_DATA_STATUS,
            data_op::OPEN_WRITER..=data_op::CLEAR | data_op::ATTACH_PROVIDER => topics::DRAW_DATA,
            _ => {
                return Err(DrawDataError::Protocol(
                    "capabilities require a new negotiation".into(),
                ));
            }
        };
        let mut client = agentio::ById::new(&self.node, self.host).req_client::<Env, Env>(topic)?;
        let reply = call(&mut client, request, deadline)?;
        if reply.host_epoch != self.capabilities.host_epoch && reply.code != data_code::STALE_EPOCH
        {
            return Err(DrawDataError::Protocol(
                "reply changed the negotiated host epoch".into(),
            ));
        }
        Ok(reply)
    }
}

fn capabilities(
    client: &Client,
    host: EndpointId,
    deadline: Instant,
) -> Result<DataCapabilities, DrawDataError> {
    let mut topic = data_topic(client, host, topics::DRAW_DATA_CAPABILITIES, deadline)?;
    let reply = call(
        &mut topic,
        &DataRequest::new(data_op::CAPABILITIES),
        deadline,
    )?;
    if reply.code != data_code::OK {
        let message = String::from_utf8_lossy(&reply.message).into_owned();
        return Err(if reply.code == data_code::UNSUPPORTED {
            DrawDataError::Unsupported(message)
        } else {
            DrawDataError::Rejected {
                code: reply.code,
                message,
            }
        });
    }
    let capabilities =
        DataCapabilities::from_json(&reply.metadata).map_err(DrawDataError::Protocol)?;
    if reply.host_epoch != capabilities.host_epoch || reply.stage != data_stage::NONE {
        return Err(DrawDataError::Protocol(
            "inconsistent capability reply".into(),
        ));
    }
    if !capabilities.schemas.contains(&SCHEMA_REVISION) {
        return Err(DrawDataError::Unsupported(
            "no common dataset schema revision".into(),
        ));
    }
    check_deadline(deadline)?;
    Ok(capabilities)
}

fn call(
    client: &mut ReqClient<Env, Env>,
    request: &DataRequest,
    deadline: Instant,
) -> Result<DataReply, DrawDataError> {
    check_deadline(deadline)?;
    let env = request.pack_checked().map_err(DrawDataError::Protocol)?;
    // A full request ring refuses the slot before anything is published.
    let sample = loop {
        match client.call(&env) {
            Err(peerbus::Error::NoFreeSlot { .. }) => {
                check_deadline(deadline)?;
                std::thread::sleep(
                    Duration::from_micros(200).min(deadline.saturating_duration_since(Instant::now())),
                );
            }
            result => break result?,
        }
    };
    let reply = DataReply::unpack_checked(sample.header().type_hash, sample.payload())
        .map_err(DrawDataError::Protocol)?;
    check_deadline(deadline)?;
    Ok(reply)
}

fn data_topic(
    client: &Client,
    host: EndpointId,
    topic: &str,
    deadline: Instant,
) -> Result<ReqClient<Env, Env>, DrawDataError> {
    use agentio::directory::RESOLUTION_TYPE_HASH;
    check_deadline(deadline)?;
    let mut resolver = client
        .agent
        .node()
        .req_client::<Env, Env>(host, agentio::RESOLUTION_TOPIC)?;
    let hash = wire_type_hash::<Env>();
    let query = ResolveRequest::Query {
        topic: topic.into(),
        exchange: ExchangeKind::ReqRes,
        request_type_hash: hash,
        response_type_hash: Some(hash),
    };
    let sample = resolver.call(&Env::new(RESOLUTION_TYPE_HASH, query.to_bytes()?))?;
    check_deadline(deadline)?;
    if sample.header().type_hash != RESOLUTION_TYPE_HASH {
        return Err(DrawDataError::Protocol(
            "unexpected directory reply type".into(),
        ));
    }
    let entry = match ResolveResponse::from_bytes(sample.payload())? {
        ResolveResponse::QueryResult {
            found: false,
            entry: None,
        } => {
            return Err(DrawDataError::Unsupported(format!(
                "host does not advertise {topic}"
            )));
        }
        ResolveResponse::QueryResult {
            found: true,
            entry: Some(entry),
        } => entry,
        _ => return Err(DrawDataError::Protocol("unexpected directory reply".into())),
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| DrawDataError::Protocol("system clock precedes Unix epoch".into()))?;
    entry.verify(
        now.as_millis()
            .try_into()
            .map_err(|_| DrawDataError::Protocol("system clock overflow".into()))?,
    )?;
    entry.validate_exchange(ExchangeKind::ReqRes, hash, Some(hash))?;
    if entry.endpoint_id() != host || entry.topic() != topic {
        return Err(DrawDataError::Protocol(
            "data topic is not owned by the selected host".into(),
        ));
    }
    check_deadline(deadline)?;
    let topic = client.agent.by_id(host)?.req_client(topic)?;
    check_deadline(deadline)?;
    Ok(topic)
}

fn deadline_after(timeout: Duration) -> Result<Instant, DrawDataError> {
    Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| DrawDataError::Protocol("data deadline overflow".into()))
}

fn check_deadline(deadline: Instant) -> Result<(), DrawDataError> {
    if Instant::now() >= deadline {
        Err(DrawDataError::Timeout)
    } else {
        Ok(())
    }
}
