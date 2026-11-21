//! ASS subtitle parsing and serialization.
//!
//! Ports the Python `parsers/` package: read `.ass` files preserving structure,
//! styles, and metadata; extract dialogue lines; strip and re-apply inline
//! override tags (`{\pos(x,y)}`, `{\an8}`, font overrides) by tracked position.
//!
//! Planned submodules: `ass_file`, `ass_parser`, `dialogue_line`, `ass_tags`.

pub mod decode;
pub mod parse;
pub mod plan;
pub mod tags;
pub mod writer;
