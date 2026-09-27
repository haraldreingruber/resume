//! Resume content model.
//!
//! The single source of truth is `content/resume.yaml` (JSON Resume schema plus
//! `x-` extension fields). With the `load` feature it is parsed, validated and
//! normalized into the [`Resume`] domain model, which every output renders from:
//! the LaTeX data macros, README.md, the plain HTML page and the 3D app.
//!
//! Without `load` only the domain types are compiled, so the wasm runtime can
//! deserialize a pre-baked [`Resume`] without shipping a YAML or Markdown parser.

mod domain;
#[cfg(feature = "load")]
mod load;
#[cfg(feature = "load")]
pub mod source;

pub use domain::*;
#[cfg(feature = "load")]
pub use load::{LoadError, load_file, load_str};
