//! Kinde JWT authentication via JWKS.

use anyhow::{anyhow, Result};
use jsonwebtoken::{decode, decode_header, Validation};
use jwks::Jwks;
use serde::Deserialize;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::instrument;

/// Expected JWT claims from Kinde access token.
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
pub struct KindeClaims {
    pub sub: String,
    pub iss: Option<String>,
    pub aud: Option<serde_json::Value>,
    pub exp: Option<u64>,
    pub nbf: Option<u64>,
}

/// Validates Kinde JWTs using JWKS from the Kinde domain.
pub struct KindeValidator {
    jwks_url: String,
    issuer: String,
    audience: Option<String>,
    jwks: Arc<RwLock<Option<Jwks>>>,
}

impl KindeValidator {
    /// Build validator for Kinde domain (e.g. `myapp` for `myapp.kinde.com`).
    pub fn new(kinde_domain: &str, audience: Option<String>) -> Self {
        let domain = kinde_domain.trim_end_matches(".kinde.com");
        let base = format!("https://{}.kinde.com", domain);
        let jwks_url = format!("{}/.well-known/jwks", base);
        let issuer = base;
        Self {
            jwks_url,
            issuer,
            audience,
            jwks: Arc::new(RwLock::new(None)),
        }
    }

    /// Fetch JWKS from Kinde (call periodically or on first use).
    #[instrument(skip(self))]
    pub async fn refresh_jwks(&self) -> Result<()> {
        let jwks = Jwks::from_jwks_url(&self.jwks_url).await?;
        let mut guard = self.jwks.write().await;
        *guard = Some(jwks);
        Ok(())
    }

    /// Ensure JWKS is loaded (refreshes if missing).
    async fn ensure_jwks(&self) -> Result<Jwks> {
        {
            let guard = self.jwks.read().await;
            if let Some(ref j) = *guard {
                return Ok(j.clone());
            }
        }
        self.refresh_jwks().await?;
        let guard = self.jwks.read().await;
        guard
            .clone()
            .ok_or_else(|| anyhow!("JWKS still missing after refresh"))
    }

    /// Validate token and return claims if valid.
    #[instrument(skip(self))]
    pub async fn validate(&self, token: &str) -> Result<KindeClaims> {
        let header = decode_header(token).map_err(|e| anyhow!("invalid JWT header: {}", e))?;
        let kid = header
            .kid
            .as_ref()
            .ok_or_else(|| anyhow!("JWT missing kid"))?;
        if header.alg != jsonwebtoken::Algorithm::RS256 {
            return Err(anyhow!("unsupported alg: {:?}", header.alg));
        }

        let jwks = self.ensure_jwks().await?;
        let jwk = jwks
            .keys
            .get(kid)
            .ok_or_else(|| anyhow!("unknown kid: {}", kid))?;

        let mut validation = Validation::new(header.alg);
        validation.validate_exp = true;
        validation.validate_nbf = true;
        validation.set_issuer(&[&self.issuer]);
        if let Some(ref aud) = self.audience {
            validation.set_audience(&[aud]);
        }

        let data = decode::<KindeClaims>(token, &jwk.decoding_key, &validation)
            .map_err(|e| anyhow!("JWT validation failed: {}", e))?;
        Ok(data.claims)
    }
}
