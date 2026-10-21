//! SSH host alias discovery.
//!
//! This is a lexical scan for selectable aliases only. It does not interpret
//! effective connection options; `ssh -G <alias>` remains the authority for that.

mod discover;
mod tokenize;

pub use discover::{discover, Context, Discovery, Host, Issue, Location, Severity};
