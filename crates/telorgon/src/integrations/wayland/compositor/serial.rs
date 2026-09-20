use std::collections::{BTreeMap, VecDeque};
use std::fmt;

use crate::integrations::wayland::compositor::{ClientId, WaylandSurfaceId};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SerialKind {
    PointerEnter,
    PointerButton,
    KeyboardEnter,
    KeyboardKey,
    TouchDown,
    DataDevice,
    XdgConfigure,
    Activation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SerialRecord {
    pub serial: u32,
    pub client: ClientId,
    pub kind: SerialKind,
    pub surface: Option<WaylandSurfaceId>,
    pub consumed: bool,
}

#[derive(Debug)]
pub struct SerialLedger {
    capacity_per_client: usize,
    records: BTreeMap<ClientId, VecDeque<SerialRecord>>,
}

impl SerialLedger {
    pub fn new(capacity_per_client: usize) -> Result<Self, SerialValidationError> {
        if capacity_per_client == 0 {
            return Err(SerialValidationError::InvalidCapacity);
        }
        Ok(Self {
            capacity_per_client,
            records: BTreeMap::new(),
        })
    }

    pub fn issue(
        &mut self,
        serial: u32,
        client: ClientId,
        kind: SerialKind,
        surface: Option<WaylandSurfaceId>,
    ) -> Result<SerialRecord, SerialValidationError> {
        if serial == 0 {
            return Err(SerialValidationError::ZeroSerial);
        }
        let records = self.records.entry(client).or_default();
        if records.iter().any(|record| record.serial == serial) {
            return Err(SerialValidationError::DuplicateSerial);
        }
        while records.len() >= self.capacity_per_client {
            records.pop_front();
        }
        let record = SerialRecord {
            serial,
            client,
            kind,
            surface,
            consumed: false,
        };
        records.push_back(record);
        Ok(record)
    }

    pub fn validate(
        &self,
        client: ClientId,
        serial: u32,
        accepted: &[SerialKind],
        surface: Option<WaylandSurfaceId>,
    ) -> Result<SerialRecord, SerialValidationError> {
        let record = self
            .records
            .get(&client)
            .and_then(|records| records.iter().find(|record| record.serial == serial))
            .copied()
            .ok_or(SerialValidationError::UnknownOrExpired)?;
        if record.consumed {
            return Err(SerialValidationError::AlreadyConsumed);
        }
        if !accepted.contains(&record.kind) {
            return Err(SerialValidationError::WrongKind);
        }
        if surface.is_some() && record.surface != surface {
            return Err(SerialValidationError::WrongSurface);
        }
        Ok(record)
    }

    pub fn consume(
        &mut self,
        client: ClientId,
        serial: u32,
        accepted: &[SerialKind],
        surface: Option<WaylandSurfaceId>,
    ) -> Result<SerialRecord, SerialValidationError> {
        self.validate(client, serial, accepted, surface)?;
        let record = self
            .records
            .get_mut(&client)
            .and_then(|records| records.iter_mut().find(|record| record.serial == serial))
            .expect("validated serial remains present");
        record.consumed = true;
        Ok(*record)
    }

    pub fn remove_client(&mut self, client: ClientId) {
        self.records.remove(&client);
    }
}

impl Default for SerialLedger {
    fn default() -> Self {
        Self::new(256).expect("default serial capacity is valid")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SerialValidationError {
    InvalidCapacity,
    ZeroSerial,
    DuplicateSerial,
    UnknownOrExpired,
    AlreadyConsumed,
    WrongKind,
    WrongSurface,
}

impl fmt::Display for SerialValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "Wayland serial validation failed: {self:?}")
    }
}

impl std::error::Error for SerialValidationError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serials_are_client_scoped_kind_checked_and_single_use() {
        let client = ClientId::from_raw(1).unwrap();
        let surface = WaylandSurfaceId::from_raw(2).unwrap();
        let mut ledger = SerialLedger::new(4).unwrap();
        ledger
            .issue(7, client, SerialKind::PointerButton, Some(surface))
            .unwrap();
        ledger
            .consume(client, 7, &[SerialKind::PointerButton], Some(surface))
            .unwrap();
        assert_eq!(
            ledger.consume(client, 7, &[SerialKind::PointerButton], Some(surface)),
            Err(SerialValidationError::AlreadyConsumed)
        );
    }
}
