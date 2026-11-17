//! English for every Portuguese message the binary writes. One slice per area, so each part of
//! the code keeps its own list; the catalog joins them.

pub const ENTRIES: &[&[(&str, &str)]] = &[CORE];

/// Messages that belong to no area in particular.
const CORE: &[(&str, &str)] = &[("o cofre não existe mais", "the vault no longer exists")];
