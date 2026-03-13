use std::str::FromStr;

use iroh::EndpointAddr;
use iroh_tickets::endpoint::EndpointTicket;

use super::error::P2pError;

pub fn create_ticket(addr: EndpointAddr) -> String {
    EndpointTicket::new(addr).to_string()
}

pub fn parse_ticket(input: &str) -> Result<EndpointAddr, P2pError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(P2pError::TicketEmpty);
    }
    let ticket = EndpointTicket::from_str(trimmed)?;
    Ok(ticket.endpoint_addr().clone())
}
