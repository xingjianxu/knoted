//! Controlled staged objects shared by the worker and the intelligence client.
//!
//! Staging is a flat key/value store: an object is addressed by an
//! [`staging::ObjectId`], never by a path. The id cannot contain a separator or
//! a `..` segment, and the store re-checks that the resolved path stays inside
//! its root, so a malformed id can never read or write outside the staging
//! tree. Writes are atomic (temporary file plus rename), which is what lets the
//! worker publish an artifact that the Python service wrote concurrently.

pub mod staging;

pub use staging::{ObjectId, Sha256Hex, StagedObject, StagingError, StagingStore};

pub mod capture;
pub mod gc;
pub mod notifications;
pub mod safe_fs;
pub mod scanner;

pub mod identity;

pub mod text;

pub mod extraction;

pub mod pdf;

pub mod office;

pub(crate) mod artifact;

pub mod browse;

pub mod download;
