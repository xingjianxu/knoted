//! Account identity and the one-time tokens that create or recover it.
//!
//! The module is split so T14/T16 can add WebAuthn ceremonies next to the token
//! lifecycle instead of growing one file: [`bootstrap`] owns the unique owner
//! and the CLI-issued tokens. No Axum type appears here, per the architecture
//! rule that `knoted-core` owns persistence.

mod bootstrap;

pub use bootstrap::{
    DomainAuthError, IssuedToken, TOKEN_TTL, TokenPurpose, TokenRecord, User, consume_bootstrap,
    ensure_owner, hash_token, issue_token, verify_token,
};
