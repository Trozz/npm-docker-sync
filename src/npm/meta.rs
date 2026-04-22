use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use time::OffsetDateTime;

pub const MARKER_KEY: &str = "npm_docker_sync";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OwnershipMarker {
    pub version: u32,
    pub container_id: String,
    pub container_name: String,
    #[serde(with = "time::serde::rfc3339")]
    pub managed_at: OffsetDateTime,
}

pub fn encode(marker: &OwnershipMarker, existing_meta: Option<&Value>) -> Value {
    let mut obj: Map<String, Value> = match existing_meta {
        Some(Value::Object(m)) => m.clone(),
        _ => Map::new(),
    };
    obj.insert(
        MARKER_KEY.to_string(),
        serde_json::to_value(marker).expect("OwnershipMarker serializes into JSON"),
    );
    Value::Object(obj)
}

pub fn decode(meta: &Value) -> Option<OwnershipMarker> {
    let obj = meta.as_object()?;
    let raw = obj.get(MARKER_KEY)?;
    serde_json::from_value::<OwnershipMarker>(raw.clone()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use time::macros::datetime;

    fn sample() -> OwnershipMarker {
        OwnershipMarker {
            version: 1,
            container_id: "7b3f".into(),
            container_name: "myapp".into(),
            managed_at: datetime!(2026-04-21 12:34:56 UTC),
        }
    }

    #[test]
    fn encode_into_fresh_meta() {
        let v = encode(&sample(), None);
        let marker = v.get("npm_docker_sync").expect("marker key present");
        assert_eq!(marker["container_id"], "7b3f");
        assert_eq!(marker["version"], 1);
    }

    #[test]
    fn encode_preserves_existing_keys() {
        let existing = json!({ "user_notes": "do not touch" });
        let v = encode(&sample(), Some(&existing));
        assert_eq!(v["user_notes"], "do not touch");
        assert!(v.get("npm_docker_sync").is_some());
    }

    #[test]
    fn encode_overwrites_stale_marker() {
        let existing = json!({ "npm_docker_sync": { "container_id": "old" } });
        let v = encode(&sample(), Some(&existing));
        assert_eq!(v["npm_docker_sync"]["container_id"], "7b3f");
    }

    #[test]
    fn decode_none_when_key_missing() {
        assert!(decode(&json!({})).is_none());
    }

    #[test]
    fn decode_round_trips() {
        let v = encode(&sample(), None);
        let got = decode(&v).expect("decoded");
        assert_eq!(got, sample());
    }

    #[test]
    fn decode_none_on_malformed_marker() {
        let v = json!({ "npm_docker_sync": "nonsense" });
        assert!(decode(&v).is_none());
    }
}
