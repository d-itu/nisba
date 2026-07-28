use std::{collections::hash_map, fmt::Display, mem::MaybeUninit, result};

use ahash::{AHashMap, AHashSet};
use thiserror::Error;

use crate::parser::{IntoSpanned as _, Spanned, ast};

use super::*;

#[derive(Error, Debug)]
pub enum ErrorKind {
    #[error("integer primitive exceeds the maximum value")]
    IntegerPrimitiveTooBig,
    #[error("duplicate definition")]
    DuplicateDefinition,
    #[error("duplicate member")]
    DuplicateMember,
    #[error("invalid varint")]
    InvalidVarint,
    #[error("invalid length type")]
    InvalidIntegerLengthType,
    #[error("invalid index type")]
    InvalidIndexType,
}

type SpannedError = Spanned<ErrorKind>;

#[derive(Error, Debug)]
pub enum Error {
    Spanned(#[from] SpannedError),
    UnknownTypeName(Vec<Spanned<Ident>>),
}

impl Display for Error {
    fn fmt(&self, _: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        todo!()
    }
}

type SpannedResult<T> = result::Result<T, SpannedError>;
type Result<T> = result::Result<T, ErrorKind>;

#[derive(Default)]
struct Arena {
    assigned: Vec<MaybeUninit<Definition>>,
    // members: Vec<Vec<Span>>,
    unassigned: Vec<Option<Spanned<Ident>>>,
}

impl Arena {
    fn allocate(&mut self, name: Spanned<Ident>) -> Handle {
        let id = self.assigned.len();
        self.assigned.push(MaybeUninit::uninit());
        self.unassigned.push(Some(name));
        Handle(id)
    }
    fn is_assigned(&self, id: Handle) -> bool {
        self.unassigned[id.0].is_none()
    }
    fn insert(&mut self, id: Handle, definition: Definition) {
        debug_assert_eq!(
            self.unassigned[id.0].as_ref().map(|x| &x.item),
            Some(definition.name().unwrap())
        );
        self.assigned[id.0].write(definition);
        self.unassigned[id.0].take();
    }
    fn append(&mut self, definition: Definition) -> Handle {
        let id = self.assigned.len();
        self.assigned.push(MaybeUninit::new(definition));
        self.unassigned.push(None);
        Handle(id)
    }
    fn finish(self) -> result::Result<Schema, Error> {
        let mut unknown = vec![];
        for x in self.unassigned.into_iter().flatten() {
            unknown.push(x);
        }
        if unknown.is_empty() {
            return Ok(Schema {
                definitions: {
                    let (ptr, len, cap) = self.assigned.into_raw_parts();
                    // TODO: cast_init
                    unsafe { Vec::from_raw_parts(ptr.cast(), len, cap) }
                },
            });
        }
        Err(Error::UnknownTypeName(unknown))
    }
}

#[derive(PartialEq, Eq, Hash)]
enum Key {
    Name(Ident),
    Anon(Anon),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct Anon {
    kind: SequenceKind,
    len_ty: LenType,
    elem_ty: Type,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SequenceKind {
    Vector,
    Stream,
}

impl SequenceKind {
    fn from_ast(value: ast::SequenceKind) -> Self {
        match value {
            ast::SequenceKind::Vector => Self::Vector,
            ast::SequenceKind::Stream => Self::Stream,
        }
    }
}

#[derive(Default)]
pub(super) struct Resolver {
    arena: Arena,
    cache: AHashMap<Key, Handle>,
}

impl Resolver {
    fn get_or_allocate(&mut self, name: Spanned<Ident>) -> Handle {
        match self.cache.entry(Key::Name(name.item.clone())) {
            hash_map::Entry::Occupied(entry) => *entry.get(),
            hash_map::Entry::Vacant(entry) => {
                let id = self.arena.allocate(name);
                entry.insert(id);
                id
            }
        }
    }

    fn get_or_define_anon(
        &mut self,
        anon @ Anon {
            kind,
            len_ty,
            elem_ty,
        }: Anon,
    ) -> Handle {
        match self.cache.entry(Key::Anon(anon)) {
            hash_map::Entry::Occupied(entry) => *entry.get(),
            hash_map::Entry::Vacant(entry) => {
                let id = self.arena.append(match kind {
                    SequenceKind::Vector => Definition::Vector(Vector { len_ty, elem_ty }),
                    SequenceKind::Stream => Definition::Stream(Stream { len_ty, elem_ty }),
                });
                entry.insert(id);
                id
            }
        }
    }

    pub fn resolve(mut self, doc: &ast::Document) -> result::Result<Schema, Error> {
        for def in &doc.definitions {
            self.resolve_def(&def.item)?;
        }
        self.arena.finish()
    }

    fn resolve_def(
        &mut self,
        ast::NamedDefinition { name, def }: &ast::NamedDefinition,
    ) -> SpannedResult<()> {
        match def {
            ast::Definition::Extern(ty) => {
                let id = self.get_or_allocate(name.clone());
                if self.arena.is_assigned(id) {
                    Err(ErrorKind::DuplicateDefinition.into_spanned(name.span))?;
                }
                self.arena.insert(
                    id,
                    match ty {
                        Some(ty) => Definition::Primitive(Primitive {
                            bit_width: ty.item.bit_width().map_err(ty.span.spanned())?,
                            name: name.item.clone(),
                        }),
                        None => Definition::Extern(name.item.clone()),
                    },
                )
            }
            ast::Definition::StructLike { kind, members } => {
                let id = self.get_or_allocate(name.clone());
                if self.arena.is_assigned(id) {
                    Err(ErrorKind::DuplicateDefinition.into_spanned(name.span))?;
                }
                let mut used_names = AHashSet::new();
                let mut resolved_members = Vec::with_capacity(members.len());
                for Spanned { item: member, .. } in members {
                    if !used_names.insert(member.name.item.clone()) {
                        Err(ErrorKind::DuplicateMember.into_spanned(member.name.span))?
                    }
                    let ty = self.resolve_type(&member.ty)?;
                    // TODO: use push_within_capacity
                    resolved_members.push(Member {
                        name: member.name.item.clone(),
                        ty,
                    });
                }
                self.arena.insert(
                    id,
                    match kind {
                        ast::StructKind::Struct => Definition::Struct(Struct {
                            name: name.item.clone(),
                            members: resolved_members.into_boxed_slice(),
                        }),
                        ast::StructKind::Packed => Definition::Packed(Packed {
                            name: name.item.clone(),
                            members: resolved_members.into_boxed_slice(),
                        }),
                    },
                )
            }
            ast::Definition::IndexedStructLike {
                kind,
                index_ty,
                members,
            } => {
                let id = self.get_or_allocate(name.clone());
                if self.arena.is_assigned(id) {
                    Err(ErrorKind::DuplicateDefinition.into_spanned(name.span))?;
                }
                let index_ty = index_ty
                    .item
                    .0
                    .index_ty()
                    .map_err(index_ty.span.spanned())?;
                let mut used_names = AHashSet::new();
                let mut resolved_members = Vec::with_capacity(members.len());
                let mut index_acc = 0;
                for Spanned { item: member, .. } in members {
                    if !used_names.insert(member.name.item.clone()) {
                        Err(ErrorKind::DuplicateMember.into_spanned(member.name.span))?
                    }
                    let ty = match &member.ty {
                        Some(ty) => self.resolve_type(ty)?,
                        None => Type::Integer {
                            signedness: Signedness::Unsigned,
                            bit_width: 0,
                        },
                    };
                    let index = match member.index {
                        Some(x) => {
                            let value = x.item.get_u64().ok_or_else(|| {
                                ErrorKind::IntegerPrimitiveTooBig.into_spanned(x.span)
                            })?;
                            index_acc = value;
                            value
                        }
                        None => index_acc,
                    };
                    index_acc += 1;
                    // TODO: use push_within_capacity
                    resolved_members.push(IndexedMember {
                        index,
                        member: Member {
                            name: member.name.item.clone(),
                            ty,
                        },
                    });
                }
                self.arena.insert(
                    id,
                    match kind {
                        ast::IndexedStructKind::Enum => Definition::Enum(Enum {
                            name: name.item.clone(),
                            index_ty,
                            members: resolved_members.into_boxed_slice(),
                        }),
                        ast::IndexedStructKind::Dict => Definition::Dict(Dict {
                            name: name.item.clone(),
                            index_ty,
                            members: resolved_members.into_boxed_slice(),
                        }),
                    },
                )
            }
        }
        Ok(())
    }

    fn resovle_seq(&mut self, seq: &ast::SequenceLike) -> SpannedResult<Handle> {
        let len_ty = seq.len_ty.resolve()?;
        let elem_ty = self.resolve_type(&seq.elem_ty)?;
        Ok(self.get_or_define_anon(Anon {
            kind: SequenceKind::from_ast(seq.kind),
            len_ty,
            elem_ty,
        }))
    }

    fn resolve_type(&mut self, ty: &Spanned<ast::Type>) -> SpannedResult<Type> {
        Ok(match &ty.item {
            ast::Type::Sequence(seq) => self.resovle_seq(seq).map(Type::Allocated)?,
            &ast::Type::VarintSigned(ast::Varint(Spanned {
                span,
                item: ast::Signed(number),
            })) => Type::Varint {
                signedness: Signedness::Signed,
                size: number.varint_size().map_err(span.spanned())?,
            },
            &ast::Type::VarintUnsigned(ast::Varint(Spanned {
                span,
                item: ast::Unsigned(number),
            })) => Type::Varint {
                signedness: Signedness::Unsigned,
                size: number.varint_size().map_err(span.spanned())?,
            },
            ast::Type::Ident(ident) => {
                Type::Allocated(self.get_or_allocate(ty.replace(ident.clone())))
            }
            ast::Type::Builtin(builtin) => builtin.resolve().map_err(ty.span.spanned())?,
        })
    }
}

impl ast::Builtin {
    fn bit_width(&self) -> Result<u16> {
        Ok(match self {
            ast::Builtin::Void => 0,
            ast::Builtin::Unsigned(ast::Unsigned(x)) | ast::Builtin::Signed(ast::Signed(x)) => {
                x.int_bit_width()?
            }
        })
    }
    fn resolve(&self) -> Result<Type> {
        let signedness = match self {
            ast::Builtin::Void | ast::Builtin::Unsigned(_) => Signedness::Unsigned,
            ast::Builtin::Signed(_) => Signedness::Signed,
        };
        self.bit_width().map(|bit_width| Type::Integer {
            signedness,
            bit_width,
        })
    }
}

impl ast::Number {
    fn get_u64(self) -> Option<u64> {
        match self {
            ast::Number::Value(x) => Some(x),
            ast::Number::Overflow => None,
        }
    }
    fn int_bit_width(self) -> Result<u16> {
        match self {
            ast::Number::Value(x) if x <= u16::MAX as _ => Ok(x as _),
            _ => Err(ErrorKind::IntegerPrimitiveTooBig),
        }
    }
    fn varint_size(self) -> Result<VarintSize> {
        Ok(match self {
            ast::Number::Value(16) => VarintSize::V16,
            ast::Number::Value(32) => VarintSize::V32,
            ast::Number::Value(64) => VarintSize::V64,
            ast::Number::Overflow => Err(ErrorKind::InvalidVarint)?,
            _ => Err(ErrorKind::InvalidIntegerLengthType)?,
        })
    }
    fn index_ty(self) -> Result<IndexType> {
        Ok(match self {
            ast::Number::Value(x) => match x {
                8 => IndexType::U8,
                16 => IndexType::U16,
                24 => IndexType::U24,
                32 => IndexType::U32,
                40 => IndexType::U40,
                48 => IndexType::U48,
                56 => IndexType::U56,
                64 => IndexType::U64,
                _ => Err(ErrorKind::InvalidIndexType)?,
            },
            ast::Number::Overflow => Err(ErrorKind::IntegerPrimitiveTooBig)?,
        })
    }
}

impl Definition {
    fn name(&self) -> Option<&Ident> {
        Some(match self {
            Definition::Vector(_) | Definition::Stream(_) => None?,
            Definition::Primitive(Primitive { name, .. })
            | Definition::Packed(Packed { name, .. })
            | Definition::Struct(Struct { name, .. })
            | Definition::Enum(Enum { name, .. })
            | Definition::Dict(Dict { name, .. })
            | Definition::Extern(name) => name,
        })
    }
}

impl Spanned<ast::LenType> {
    fn resolve(&self) -> SpannedResult<LenType> {
        match &self.item {
            ast::LenType::Fixed(ast::Unsigned(number)) => {
                LenType::from_bit_width(number.int_bit_width().map_err(self.span.spanned())?)
            }
            &ast::LenType::Variant(ast::Varint(Spanned {
                item: ast::Unsigned(x),
                span,
            })) => LenType::from_varint_size(x.varint_size().map_err(span.spanned())?),
        }
        .map_err(self.span.spanned())
    }
}

impl LenType {
    fn from_bit_width(x: u16) -> Result<Self> {
        Ok(match x {
            8 => Self::U8,
            16 => Self::U16,
            24 => Self::U24,
            32 => Self::U32,
            40 => Self::U40,
            48 => Self::U48,
            56 => Self::U56,
            64 => Self::U64,
            _ => Err(ErrorKind::InvalidIntegerLengthType)?,
        })
    }
    fn from_varint_size(x: VarintSize) -> Result<Self> {
        Ok(match x {
            VarintSize::V16 => Self::V16,
            VarintSize::V32 => Self::V32,
            VarintSize::V64 => Self::V64,
        })
    }
}
