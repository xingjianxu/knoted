//! Tags and stable, typed entity associations. SQL stays in repository.
mod repository;
pub mod service;

pub(crate) use repository::transfer_file_metadata;
