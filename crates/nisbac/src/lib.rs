use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error(transparent)]
    ParseError(#[from] pest::error::Error<ast::Rule>),
    #[error("Multiple definitions for identifier '{0}'")]
    MultipleDefinitions(Ident),
    #[error("Duplicate member name '{0}'")]
    DuplicateMemberName(Ident),
    #[error("Discriminant assigned more than once")]
    DiscriminantAssignedMoreThanOnce,
    #[error("Discriminant has unsupported bit width {0}")]
    InvalidDiscriminantBitWidth(u16),
    #[error("Discriminant value out of range")]
    DiscriminantValueOutOfRange,
    #[error("Size not aligned to byte")]
    NotByteAligned,
    #[error("Size must be known at compilation time")]
    UnknownSize,
    #[error("Field containing unknown type")]
    UnknownType(Vec<Ident>),
    #[error("Unsupported varint bit width")]
    UnsupportedVarintBitWidth,
    #[error("Length integer exceeds maximum bit width of 64")]
    LengthTypeIntegerTooBig,
}

pub type Ident = smol_str::SmolStr;

pub mod ast;
pub mod back;
pub mod schema;
