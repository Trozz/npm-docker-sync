use crate::docker::spec::ContainerSpec;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intent {
    Upsert {
        container_id: String,
        spec: ContainerSpec,
    },
    Remove {
        container_id: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docker::spec::ContainerSpec;

    #[test]
    fn remove_intent_carries_container_id() {
        let i = Intent::Remove {
            container_id: "abc".into(),
        };
        match i {
            Intent::Remove { container_id } => assert_eq!(container_id, "abc"),
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn upsert_intent_carries_spec() {
        let spec = ContainerSpec::stub("abc", "app.example.com");
        let i = Intent::Upsert {
            container_id: "abc".into(),
            spec: spec.clone(),
        };
        match i {
            Intent::Upsert {
                container_id,
                spec: got,
            } => {
                assert_eq!(container_id, "abc");
                assert_eq!(got, spec);
            }
            _ => panic!("wrong variant"),
        }
    }
}
