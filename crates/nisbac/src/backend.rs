#[derive(Clone, Copy)]
pub enum CodeGenKind {
    Encode,
    Decode,
}

pub mod rust;