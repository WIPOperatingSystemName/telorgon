//! Untrusted portal restore hints. A decoded record never confers capture authority:
//! the host must resolve its opaque grant against its own authorization/revocation store.
use zbus::zvariant::{OwnedValue, Value};

const VENDOR: &str = "telorgon";
const VERSION: u32 = 1;
const MAX_TEXT: usize = 512;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RestoreSource {
    pub kind: u32,
    /// Host-defined persistent identity; never a reusable PipeWire node ID.
    pub identity: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct RestoreData {
    pub app_id: String,
    /// Random 256-bit grant identifier, lowercase hex. Knowledge alone is insufficient:
    /// the host's stored grant must also match the app, sources and effective persistence.
    pub grant_id: String,
    pub sources: Vec<RestoreSource>,
}

impl RestoreData {
    pub fn valid_for(&self, app_id: &str, types: u32, multiple: bool) -> bool {
        self.app_id == app_id
            && text(&self.app_id)
            && self.grant_id.len() == 64
            && self
                .grant_id
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            && !self.sources.is_empty()
            && self.sources.len() <= if multiple { super::MAX_STREAMS } else { 1 }
            && self.sources.iter().enumerate().all(|(index, source)| {
                matches!(source.kind, 1 | 2 | 4)
                    && source.kind & types != 0
                    && text(&source.identity)
                    && !self.sources[..index].contains(source)
            })
    }

    /// Inspect borrowed values before allocating owned text. Foreign versions, malformed
    /// values and incompatible hints are ignored by callers, returning to normal consent.
    pub fn decode(value: &OwnedValue, app_id: &str, types: u32, multiple: bool) -> Option<Self> {
        let Value::Structure(outer) = &**value else {
            return None;
        };
        let [
            Value::Str(vendor),
            Value::U32(version),
            Value::Value(payload),
        ] = outer.fields()
        else {
            return None;
        };
        if vendor.as_str() != VENDOR || *version != VERSION {
            return None;
        }
        let Value::Structure(payload) = &**payload else {
            return None;
        };
        let [Value::Str(app), Value::Str(grant), Value::Array(sources)] = payload.fields() else {
            return None;
        };
        if app.as_str() != app_id
            || !text(app.as_str())
            || grant.len() != 64
            || sources.is_empty()
            || sources.len() > if multiple { super::MAX_STREAMS } else { 1 }
        {
            return None;
        }
        // Bound every field before cloning any string from the untrusted request.
        for source in sources.inner() {
            let Value::Structure(source) = source else {
                return None;
            };
            let [Value::U32(kind), Value::Str(identity)] = source.fields() else {
                return None;
            };
            if !matches!(kind, 1 | 2 | 4) || *kind & types == 0 || !text(identity.as_str()) {
                return None;
            }
        }
        let sources = sources
            .inner()
            .iter()
            .map(|source| {
                let Value::Structure(source) = source else {
                    unreachable!()
                };
                let [Value::U32(kind), Value::Str(identity)] = source.fields() else {
                    unreachable!()
                };
                RestoreSource {
                    kind: *kind,
                    identity: identity.as_str().to_owned(),
                }
            })
            .collect();
        let data = Self {
            app_id: app.as_str().into(),
            grant_id: grant.as_str().into(),
            sources,
        };
        data.valid_for(app_id, types, multiple).then_some(data)
    }

    pub fn encode(&self) -> Option<OwnedValue> {
        if !self.valid_for(&self.app_id, 7, true) {
            return None;
        }
        let sources: Vec<_> = self
            .sources
            .iter()
            .map(|s| (s.kind, s.identity.as_str()))
            .collect();
        Value::from((
            VENDOR,
            VERSION,
            Value::from((self.app_id.as_str(), self.grant_id.as_str(), sources)),
        ))
        .try_into()
        .ok()
    }
}

fn text(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_TEXT && !value.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn record() -> RestoreData {
        RestoreData {
            app_id: "org.example.Recorder".into(),
            grant_id: "a1".repeat(32),
            sources: vec![
                RestoreSource {
                    kind: 1,
                    identity: "display:manufacturer:model:serial:connector".into(),
                },
                RestoreSource {
                    kind: 2,
                    identity: "host-issued-window-identity".into(),
                },
            ],
        }
    }
    #[test]
    fn roundtrip_is_versioned_and_bound_to_app_sources_and_cardinality() {
        let record = record();
        let encoded = record.encode().unwrap();
        assert_eq!(encoded.value_signature().to_string(), "(suv)");
        assert_eq!(
            RestoreData::decode(&encoded, &record.app_id, 3, true),
            Some(record.clone())
        );
        assert!(RestoreData::decode(&encoded, "other.app", 3, true).is_none());
        assert!(RestoreData::decode(&encoded, &record.app_id, 1, true).is_none());
        assert!(RestoreData::decode(&encoded, &record.app_id, 3, false).is_none());
    }
    #[test]
    fn malformed_foreign_and_oversized_payloads_are_ignored() {
        for value in [
            OwnedValue::from(3u32),
            Value::from(("other", VERSION, Value::from(1u32)))
                .try_into()
                .unwrap(),
            Value::from((VENDOR, VERSION + 1, Value::from(1u32)))
                .try_into()
                .unwrap(),
            Value::from((
                VENDOR,
                VERSION,
                Value::from((
                    "org.example.Recorder",
                    "a1".repeat(32),
                    vec![(1u32, "x".repeat(513))],
                )),
            ))
            .try_into()
            .unwrap(),
            Value::from((
                VENDOR,
                VERSION,
                Value::from((
                    "org.example.Recorder",
                    "a1".repeat(32),
                    vec![(1u32, "x"); 9],
                )),
            ))
            .try_into()
            .unwrap(),
        ] {
            assert!(RestoreData::decode(&value, "org.example.Recorder", 7, true).is_none());
        }
    }
    #[test]
    fn invalid_tokens_duplicates_and_empty_records_cannot_be_issued() {
        for case in 0..6 {
            let mut record = record();
            match case {
                0 => record.grant_id = "Z".repeat(64),
                1 => record.sources.push(record.sources[0].clone()),
                2 => record.sources.clear(),
                3 => record.sources[0].kind = 3,
                4 => record.sources[0].identity = "bad\nidentity".into(),
                _ => record.app_id.clear(),
            }
            assert!(record.encode().is_none());
        }
    }
}
