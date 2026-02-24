use std::{
    collections::{BTreeSet, HashMap},
    mem,
};

use crate::{
    Error,
    ast::{self, Signed, TypeDef, TypeDefValue, Unsigned, VarintSigned, VarintUnsigned},
};

type Identifier = String;

#[derive(Debug)]
pub struct Schema {
    definitions: Vec<Definition>,
}

impl Schema {
    fn new() -> Self {
        Self {
            definitions: Vec::new(),
        }
    }
    fn define(&mut self, definition: Definition) -> Handle {
        let handle = Handle(self.definitions.len());
        self.definitions.push(definition);
        handle
    }
    fn validate(&self) -> Result<(), Error> {
        for definition in &self.definitions {
            definition.validate(self)?;
        }
        Ok(())
    }
    pub fn definitions(&self) -> &[Definition] {
        &self.definitions
    }
    fn resolve(
        &mut self,
        ty: &mut ast::Type,
        resolved: &HashMap<Identifier, Handle>,
    ) -> Result<Option<Type>, Error> {
        Ok(match ty {
            ast::Type::Unresolved(unresolved) => match unresolved {
                ast::UnresolvedType::Ident(ident) => {
                    if let Some(&handle) = resolved.get(ident) {
                        let res = Type::Definition(handle);
                        *ty = ast::Type::Resolved(Type::Definition(handle));
                        Some(res)
                    } else {
                        None
                    }
                }
                &mut ast::UnresolvedType::Signed(Signed(bits)) => {
                    let res = Type::Integer(Integer {
                        bits,
                        signed: Signedness::Signed,
                    });
                    *ty = ast::Type::Resolved(res);
                    Some(res)
                }
                &mut ast::UnresolvedType::Unsigned(Unsigned(bits)) => {
                    let res = Type::Integer(Integer {
                        bits,
                        signed: Signedness::Unsigned,
                    });
                    *ty = ast::Type::Resolved(res);
                    Some(res)
                }
                ast::UnresolvedType::Void => {
                    let res = Type::Integer(Integer {
                        bits: 0,
                        signed: Signedness::Unsigned,
                    });
                    *ty = ast::Type::Resolved(res);
                    Some(res)
                }
                &mut ast::UnresolvedType::VarintSigned(VarintSigned(bits)) => {
                    let res = Type::Varint(Integer {
                        bits,
                        signed: Signedness::Signed,
                    });
                    *ty = ast::Type::Resolved(res);
                    Some(res)
                }
                &mut ast::UnresolvedType::VarintUnsigned(VarintUnsigned(bits)) => {
                    let res = Type::Varint(Integer {
                        bits,
                        signed: Signedness::Unsigned,
                    });
                    *ty = ast::Type::Resolved(res);
                    Some(res)
                }
                ast::UnresolvedType::Array {
                    kind,
                    len_type,
                    item_type,
                } => {
                    if let Some(res) = self.resolve(item_type, resolved)? {
                        let arr = match kind {
                            ast::ArrayKind::Vector => Definition::Vector(Vector {
                                len_type: LenType::from_ast(*len_type),
                                element_type: res,
                            }),
                            ast::ArrayKind::Stream => Definition::Stream(Stream {
                                len_type: LenType::from_ast(*len_type),
                                element_type: res,
                            }),
                        };
                        let res = Type::Definition(
                            if let Some(idx) = self.definitions.iter().position(|x| x == &arr) {
                                Handle(idx)
                            } else {
                                self.define(arr)
                            },
                        );
                        *ty = ast::Type::Resolved(res);
                        Some(res)
                    } else {
                        None
                    }
                }
            },
            &mut ast::Type::Resolved(res) => Some(res),
        })
    }
    pub fn from_ast(mut typedefs: Vec<TypeDef>) -> Result<Self, Error> {
        let mut result = Self::new();
        let mut resolved = HashMap::new();

        while resolved.len() != typedefs.len() {
            let mut iter_resolved = 0;
            'iter: for typedef in &mut typedefs {
                if let TypeDef::Unresolved { name, value } = typedef {
                    match value {
                        TypeDefValue::Primitive(ty) => {
                            let name = name.clone();
                            let rhs = match ty {
                                &mut ast::Primitive::Signed(Signed(bits)) => {
                                    Primitive { name, bits }
                                }
                                &mut ast::Primitive::Unsigned(Unsigned(bits)) => {
                                    Primitive { name, bits }
                                }
                                ast::Primitive::Void => Primitive { name, bits: 0 },
                            };
                            let handle = result.define(Definition::Primitive(rhs));
                            iter_resolved += 1;
                            if let TypeDef::Unresolved { name, .. } =
                                mem::replace(typedef, TypeDef::Resolved(handle))
                            {
                                if resolved.insert(name.clone(), handle).is_some() {
                                    Err(Error::MultipleDefinitions(name))?
                                }
                            };
                        }
                        TypeDefValue::Tuple { kind, members } => {
                            for member in &mut members[..] {
                                if let None = result.resolve(&mut member.ty, &resolved)? {
                                    continue 'iter;
                                }
                            }

                            let mut resolved_members = Vec::with_capacity(members.len());
                            for member in members {
                                match member.ty {
                                    ast::Type::Unresolved(_) => unreachable!(),
                                    ast::Type::Resolved(ty) => resolved_members.push(Member {
                                        name: member.name.clone(),
                                        ty,
                                    }),
                                }
                            }
                            let handle = match kind {
                                ast::TupleKind::Packed => {
                                    result.define(Definition::Packed(Packed {
                                        name: name.clone(),
                                        members: resolved_members,
                                    }))
                                }
                                ast::TupleKind::Struct => {
                                    result.define(Definition::Struct(Struct {
                                        name: name.clone(),
                                        members: resolved_members,
                                    }))
                                }
                            };
                            iter_resolved += 1;
                            if let TypeDef::Unresolved { name, .. } =
                                mem::replace(typedef, TypeDef::Resolved(handle))
                            {
                                if resolved.insert(name.clone(), handle).is_some() {
                                    Err(Error::MultipleDefinitions(name))?
                                }
                            };
                        }
                        &mut TypeDefValue::Union {
                            ref kind,
                            tag: ast::Unsigned(tag_bits),
                            ref mut members,
                        } => {
                            if tag_bits % 8 != 0 {
                                Err(Error::InvalidTagBits(tag_bits))?
                            }
                            for member in &mut members[..] {
                                if let None = result.resolve(
                                    member.ty.get_or_insert(ast::Type::Unresolved(
                                        ast::UnresolvedType::Void,
                                    )),
                                    &resolved,
                                )? {
                                    continue 'iter;
                                }
                            }
                            let mut resolved_members = Vec::with_capacity(members.len());
                            let mut assigned_tags = BTreeSet::new();
                            let tag_max = match kind {
                                ast::UnionKind::Enum => (1 << tag_bits) - 1,
                                ast::UnionKind::Dict => tag_bits as _,
                            };
                            let mut tag_counter = match kind {
                                ast::UnionKind::Enum => 0,
                                ast::UnionKind::Dict => 1,
                            };
                            for member in members {
                                let ty = member.ty.as_ref().unwrap().resolved().unwrap();
                                let tag = match member.tag {
                                    Some(x) => {
                                        let result = x.max(tag_counter);
                                        tag_counter = result;
                                        result
                                    }
                                    None => tag_counter,
                                };
                                if !assigned_tags.insert(tag) {
                                    Err(Error::TagAssignedMoreThanOnce)?
                                }
                                if tag > tag_max {
                                    Err(Error::TagValueOutOfRange)?
                                }
                                resolved_members.push(TaggedMember {
                                    name: member.name.clone(),
                                    tag,
                                    ty,
                                });
                                tag_counter += 1;
                            }
                            let handle = match kind {
                                ast::UnionKind::Enum => result.define(Definition::Enum(Enum {
                                    name: name.clone(),
                                    tag_bits,
                                    members: resolved_members,
                                })),
                                ast::UnionKind::Dict => result.define(Definition::Dict(Dict {
                                    name: name.clone(),
                                    tag_bits,
                                    members: resolved_members,
                                })),
                            };
                            iter_resolved += 1;
                            if let TypeDef::Unresolved { name, .. } =
                                mem::replace(typedef, TypeDef::Resolved(handle))
                            {
                                if resolved.insert(name.clone(), handle).is_some() {
                                    Err(Error::MultipleDefinitions(name))?
                                }
                            };
                        }
                    }
                }
            }
            if iter_resolved == 0 {
                Err(Error::UnresolvedReference)?
            }
        }

        result.validate()?;
        Ok(result)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Handle(usize);

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Type {
    Integer(Integer),
    Varint(Integer),
    Definition(Handle),
}

#[derive(Clone, Copy, PartialEq)]
enum BitSize {
    Fixed(u16),
    Variable,
}

impl BitSize {
    fn validate(self) -> Result<(), Error> {
        match self {
            Self::Fixed(x) => {
                if x % 8 == 0 {
                    Ok(())
                } else {
                    Err(Error::ElementNotByteAligned)
                }
            }
            Self::Variable => Err(Error::VariableSize),
        }
    }
}

impl Type {
    fn bit_size(self, schema: &Schema) -> BitSize {
        match self {
            Self::Integer(Integer { bits, .. }) => BitSize::Fixed(bits),
            Self::Varint(_) => BitSize::Variable,
            Self::Definition(Handle(idx)) => schema.definitions[idx].bit_size(schema),
        }
    }
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

impl PartialEq for Definition {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Vector(l), Self::Vector(r)) => l == r,
            (Self::Stream(l), Self::Stream(r)) => l == r,
            _ => false,
        }
    }
}

impl Definition {
    fn bit_size(&self, schema: &Schema) -> BitSize {
        match self {
            &Self::Primitive(Primitive { bits, .. }) => BitSize::Fixed(bits),
            Self::Vector(_) | Self::Stream(_) | Self::Struct(_) => BitSize::Variable,
            Self::Packed(packed) => packed.bit_size(schema),
            Self::Enum(e) => e.bit_size(schema),
            Self::Dict(dict) => dict.bit_size(schema),
        }
    }
    fn validate(&self, schema: &Schema) -> Result<(), Error> {
        match self {
            Definition::Vector(Vector { element_type, .. }) => {
                element_type.bit_size(schema).validate()
            }
            Definition::Packed(packed) => packed.bit_size(schema).validate(),
            _ => Ok(()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Integer {
    bits: u16,
    signed: Signedness,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Signedness {
    Signed,
    Unsigned,
}

#[derive(Debug, PartialEq)]
pub struct LenType {
    pub bits: u16,
    pub format: LenTypeFormat,
}

impl LenType {
    fn from_ast(x: ast::LenType) -> Self {
        match x {
            ast::LenType::Unsigned(Unsigned(bits)) => LenType {
                bits,
                format: LenTypeFormat::Fixed,
            },
            ast::LenType::VarintUnsigned(VarintUnsigned(bits)) => LenType {
                bits,
                format: LenTypeFormat::Varint,
            },
        }
    }
}

#[derive(Debug, PartialEq)]
pub enum LenTypeFormat {
    Fixed,
    Varint,
}

#[derive(Debug)]
pub struct Primitive {
    pub name: Identifier,
    pub bits: u16,
}

#[derive(Debug, PartialEq)]
pub struct Vector {
    pub len_type: LenType,
    pub element_type: Type,
}

#[derive(Debug, PartialEq)]
pub struct Stream {
    pub len_type: LenType,
    pub element_type: Type,
}

#[derive(Debug)]
pub struct Packed {
    pub name: Identifier,
    pub members: Vec<Member>,
}

impl Packed {
    fn bit_size(&self, schema: &Schema) -> BitSize {
        let mut sum = 0;
        for member in &self.members {
            match member.ty.bit_size(schema) {
                BitSize::Fixed(size) => sum += size,
                BitSize::Variable => return BitSize::Variable,
            }
        }
        BitSize::Fixed(sum)
    }
}

#[derive(Debug)]
pub struct Struct {
    pub name: Identifier,
    pub members: Vec<Member>,
}

#[derive(Debug)]
pub struct Member {
    pub name: Identifier,
    pub ty: Type,
}

#[derive(Debug)]
pub struct Enum {
    pub name: Identifier,
    pub tag_bits: u16,
    pub members: Vec<TaggedMember>,
}

impl Enum {
    fn member_bit_size(&self, schema: &Schema) -> BitSize {
        let mut members = self.members.iter();
        match members.next() {
            Some(member) => match member.ty.bit_size(schema) {
                BitSize::Fixed(size) => {
                    for member in members {
                        match member.ty.bit_size(schema) {
                            BitSize::Fixed(x) => {
                                if x != size {
                                    return BitSize::Variable;
                                }
                            }
                            BitSize::Variable => return BitSize::Variable,
                        }
                    }
                    BitSize::Fixed(size)
                }
                BitSize::Variable => BitSize::Variable,
            },
            None => BitSize::Fixed(0),
        }
    }
    fn bit_size(&self, schema: &Schema) -> BitSize {
        match self.member_bit_size(schema) {
            BitSize::Fixed(size) => BitSize::Fixed(size + self.tag_bits),
            BitSize::Variable => BitSize::Variable,
        }
    }
}

#[derive(Debug)]
pub struct Dict {
    pub name: Identifier,
    pub tag_bits: u16,
    pub members: Vec<TaggedMember>,
}

impl Dict {
    fn bit_size(&self, schema: &Schema) -> BitSize {
        if self
            .members
            .iter()
            .all(|member| member.ty.bit_size(schema) == BitSize::Fixed(0))
        {
            BitSize::Fixed(self.tag_bits)
        } else {
            BitSize::Variable
        }
    }
}

#[derive(Debug)]
pub struct TaggedMember {
    pub name: Identifier,
    pub tag: u64,
    pub ty: Type,
}
