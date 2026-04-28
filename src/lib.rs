//! grimoire — declarative state for personal Linux desktops.
//!
//! Public API surface used by the binary and by tests. The binary glues these
//! pieces together via the `cli` module.

pub mod cast;
pub mod cli;
pub mod compose;
pub mod config;
pub mod constraint;
pub mod distro;
pub mod grimoire;
pub mod manifest;
pub mod resolve;
pub mod scry;
pub mod shell;
pub mod spell;
pub mod state;
pub mod system_deps;
pub mod upgrade;
pub mod version;

pub use crate::grimoire::{Grimoire, Source, SpellEntry};
pub use crate::manifest::Manifest;
pub use crate::spell::{Cast, Channel, ChannelType, Parameter, ParameterKind, Spell, VersionHint};
