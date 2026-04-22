use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerSpec {
    pub id: String,
    pub name: String,
    pub url: String,
    pub port: u16,
    pub scheme: Scheme,
    pub ssl: bool,
    pub websockets: bool,
    pub block_exploits: bool,
    pub network_aliases: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scheme {
    Http,
    Https,
}

#[allow(clippy::derivable_impls)]
impl Default for Scheme {
    fn default() -> Self {
        Scheme::Http
    }
}

impl std::str::FromStr for Scheme {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.eq_ignore_ascii_case("https") {
            Ok(Scheme::Https)
        } else if s.eq_ignore_ascii_case("http") {
            Ok(Scheme::Http)
        } else {
            Err(())
        }
    }
}

impl ContainerSpec {
    #[cfg(test)]
    pub fn stub(id: &str, url: &str) -> Self {
        Self {
            id: id.into(),
            name: "stub".into(),
            url: url.into(),
            port: 80,
            scheme: Scheme::Http,
            ssl: true,
            websockets: true,
            block_exploits: true,
            network_aliases: BTreeMap::new(),
        }
    }
}
