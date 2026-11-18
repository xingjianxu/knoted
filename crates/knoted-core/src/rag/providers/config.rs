use std::{collections::BTreeMap, fmt, net::IpAddr, path::PathBuf};

use serde::Deserialize;
use url::{Host, Url};

use super::ProviderError;

/// Model services configured by the instance operator. Local is selected by
/// default, and another provider is used only for its exact configured name.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ChatProvidersConfig {
    pub local: Option<ProviderConfig>,
    pub external: BTreeMap<String, ProviderConfig>,
}

#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderConfig {
    pub base_url: String,
    pub model: String,
    pub credential_file: PathBuf,
}

impl fmt::Debug for ProviderConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // A malformed config can contain URL userinfo, so avoid formatting the
        // raw values even before validation.
        f.debug_struct("ProviderConfig").finish_non_exhaustive()
    }
}

impl ChatProvidersConfig {
    pub(crate) fn validate(&self) -> Result<(), ProviderError> {
        if let Some(local) = &self.local {
            local.api_url(true)?;
        }
        for (name, spec) in &self.external {
            if !name
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_alphanumeric)
                || name == "local"
                || !name
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
            {
                return Err(ProviderError::InvalidConfig);
            }
            spec.api_url(false)?;
        }
        Ok(())
    }
}

impl ProviderConfig {
    pub(super) fn api_url(&self, local: bool) -> Result<Url, ProviderError> {
        let mut url = Url::parse(&self.base_url).map_err(|_| ProviderError::InvalidConfig)?;
        let loopback = match url.host() {
            Some(Host::Ipv4(ip)) => IpAddr::V4(ip).is_loopback(),
            Some(Host::Ipv6(ip)) => IpAddr::V6(ip).is_loopback(),
            _ => false,
        };
        let valid_scheme = url.scheme() == "https" || url.scheme() == "http" && loopback;
        if !valid_scheme
            || (local && !loopback)
            || url.host().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || !self.credential_file.is_absolute()
            || self.model.is_empty()
            || self.model.trim() != self.model
            || self.model.chars().any(char::is_control)
        {
            return Err(ProviderError::InvalidConfig);
        }

        let path = url.path().trim_end_matches('/');
        let api_path = if path.ends_with("/v1") {
            format!("{path}/")
        } else {
            format!("{path}/v1/")
        };
        url.set_path(&api_path);
        Ok(url)
    }
}
