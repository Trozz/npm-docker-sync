use std::collections::BTreeMap;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum TokenError {
    #[error("no Cloudflare token for domain {0} and no global fallback")]
    NotFound(String),
}

pub struct TokenResolver {
    global: Option<String>,
    per_domain: BTreeMap<String, String>,
}

impl TokenResolver {
    pub fn new(global: Option<String>, per_domain: BTreeMap<String, String>) -> Self {
        Self { global, per_domain }
    }

    pub fn for_domain(&self, domain: &str) -> Result<&str, TokenError> {
        if let Some(t) = self.per_domain.get(domain) {
            return Ok(t);
        }
        let best = self
            .per_domain
            .iter()
            .filter(|(k, _)| domain == k.as_str() || domain.ends_with(&format!(".{k}")))
            .max_by_key(|(k, _)| k.len())
            .map(|(_, v)| v.as_str());
        if let Some(t) = best {
            return Ok(t);
        }
        self.global
            .as_deref()
            .ok_or_else(|| TokenError::NotFound(domain.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn falls_back_to_global() {
        let r = TokenResolver::new(Some("global".into()), BTreeMap::new());
        assert_eq!(r.for_domain("any.example.com").unwrap(), "global");
    }

    #[test]
    fn prefers_exact_domain_match() {
        let mut map = BTreeMap::new();
        map.insert("example.com".into(), "scoped".into());
        let r = TokenResolver::new(Some("global".into()), map);
        assert_eq!(r.for_domain("example.com").unwrap(), "scoped");
    }

    #[test]
    fn matches_parent_domain_suffix() {
        let mut map = BTreeMap::new();
        map.insert("example.com".into(), "scoped".into());
        let r = TokenResolver::new(None, map);
        assert_eq!(r.for_domain("app.example.com").unwrap(), "scoped");
    }

    #[test]
    fn errors_without_any_match() {
        let r = TokenResolver::new(None, BTreeMap::new());
        assert!(matches!(
            r.for_domain("x.com"),
            Err(TokenError::NotFound(_))
        ));
    }
}
