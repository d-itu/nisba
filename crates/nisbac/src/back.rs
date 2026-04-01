#[derive(Clone, Copy, PartialEq)]
pub enum CodeGenKind {
    Encode,
    Decode,
}

pub mod rust;