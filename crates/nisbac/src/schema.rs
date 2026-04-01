use std::fmt::{self, Display, Formatter};

use crate::{Ident, parser::ast};

#[derive(Debug)]
pub struct Schema {
    pub definitions: Vec<Definition>,
}

#[derive(Debug)]
pub enum Definition {
    Primitive(Primitive),
    Vector(Vector),
    Stream(Stream),
    Packed(Packed),
    Struct(Struct),
    Enum(Enum),
    Dict(Dict),
}

#[derive(Debug)]
pub struct Primitive {
    pub name: Ident,
    pub bit_width: u16,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum LenType {
    U8 = 1,
    U16,
    U24,
    U32,
    U40,
    U48,
    U56,
    U64,

    V16 = 8 + 2,
    V32 = 8 + 4,
    V64 = 8 + 8,
}

pub enum Sequence {
    Vector(Vector),
    Stream(Stream),
}

#[derive(Debug)]
pub struct Vector {
    pub len_ty: LenType,
    pub elem_ty: Type,
}

#[derive(Debug)]
pub struct Stream {
    pub len_ty: LenType,
    pub elem_ty: Type,
}

#[derive(Debug)]
pub struct Packed {
    pub name: Ident,
    pub members: Box<[Member]>,
}

#[derive(Debug)]
pub struct Struct {
    pub name: Ident,
    pub members: Box<[Member]>,
}

#[derive(Debug)]
pub struct Enum {
    pub name: Ident,
    pub index_ty: IndexType,
    pub members: Box<[IndexedMember]>,
}

#[derive(Debug)]
pub struct Dict {
    pub name: Ident,
    pub index_ty: IndexType,
    pub members: Box<[IndexedMember]>,
}

#[derive(Debug)]
pub struct Member {
    pub name: Ident,
    pub ty: Type,
}

#[derive(Debug)]
pub struct IndexedMember {
    pub index: u64,
    pub member: Member,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Type {
    Allocated(Handle),
    Integer {
        signedness: Signedness,
        bit_width: u16,
    },
    Varint {
        signedness: Signedness,
        size: VarintSize,
    },
}

#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum IndexType {
    U8 = 1,
    U16,
    U24,
    U32,
    U40,
    U48,
    U56,
    U64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum VarintSize {
    V16 = 2,
    V32 = 4,
    V64 = 8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Signedness {
    Signed,
    Unsigned,
}

#[must_use]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Handle(pub usize);

impl Display for Handle {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

pub fn resolve(doc: &ast::Document) -> Result<Schema, resolver::Error> {
    let resolver = resolver::Resolver::default();
    resolver.resolve(doc)
}

pub struct Validated {
    pub schema: Schema,
    pub bit_width: Box<[validator::BitWidth]>,
    pub has_lifetime: Box<[bool]>,
}

pub fn validate(schema: Schema) -> Result<Validated, validator::Error> {
    let mut validator = validator::Validator::new(schema.definitions.len());
    validator.validate(&schema)?;
    Ok(Validated {
        schema,
        bit_width: unsafe { validator.bit_width.assume_init() },
        has_lifetime: validator.has_lifetime,
    })
}

pub mod resolver;
pub mod validator;
