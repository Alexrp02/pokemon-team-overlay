use std::{fmt, io, time::Duration};

use iroh::endpoint::{BindError, ClosedStream, ConnectError, ConnectionError, ReadToEndError, WriteError};
use iroh_tickets::ParseError as TicketParseError;
use serde_json::Error as JsonError;

#[derive(Debug)]
pub enum P2pError {
    EndpointBind(BindError),
    EndpointOnlineTimeout(Duration),
    TicketParse(TicketParseError),
    TicketEmpty,
    Connect(ConnectError),
    OpenUni(ConnectionError),
    AcceptUni(ConnectionError),
    Read(ReadToEndError),
    Write(WriteError),
    Finish(ClosedStream),
    Json(JsonError),
    Input(io::Error),
}

impl fmt::Display for P2pError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            P2pError::EndpointBind(err) => write!(f, "failed to bind iroh endpoint: {}", err),
            P2pError::EndpointOnlineTimeout(timeout) => {
                write!(f, "timed out waiting for iroh to come online after {:?}", timeout)
            }
            P2pError::TicketParse(err) => write!(f, "failed to parse ticket: {}", err),
            P2pError::TicketEmpty => write!(f, "ticket input was empty"),
            P2pError::Connect(err) => write!(f, "failed to connect to peer: {}", err),
            P2pError::OpenUni(err) => write!(f, "failed to open send stream: {}", err),
            P2pError::AcceptUni(err) => write!(f, "failed to accept recv stream: {}", err),
            P2pError::Read(err) => write!(f, "failed to read from peer: {}", err),
            P2pError::Write(err) => write!(f, "failed to write to peer: {}", err),
            P2pError::Finish(err) => write!(f, "failed to finish send stream: {}", err),
            P2pError::Json(err) => write!(f, "failed to serialize/deserialize json: {}", err),
            P2pError::Input(err) => write!(f, "failed to read ticket input: {}", err),
        }
    }
}

impl std::error::Error for P2pError {}

impl From<BindError> for P2pError {
    fn from(err: BindError) -> Self {
        Self::EndpointBind(err)
    }
}

impl From<TicketParseError> for P2pError {
    fn from(err: TicketParseError) -> Self {
        Self::TicketParse(err)
    }
}

impl From<ConnectError> for P2pError {
    fn from(err: ConnectError) -> Self {
        Self::Connect(err)
    }
}

impl From<ReadToEndError> for P2pError {
    fn from(err: ReadToEndError) -> Self {
        Self::Read(err)
    }
}

impl From<JsonError> for P2pError {
    fn from(err: JsonError) -> Self {
        Self::Json(err)
    }
}
