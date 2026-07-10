use std::{
    mem::MaybeUninit,
    ops::{Add, AddAssign},
    result,
};

use ahash::AHashSet;
use thiserror::Error;

use super::*;

#[derive(Error, Debug)]
pub enum Error {
    #[error("invalid type reference")]
    HandleOutOfRange,
    #[error("recursive definition")]
    RecursiveDefinition,
    #[error("fixed-sized type required")]
    RequiresFixedSize,
    #[error("byte-aligned type required")]
    RequiresByteAligned,
    #[error("member index out of range")]
    IndexOutOfRange,
    #[error("member has duplicate index")]
    DuplicateIndex,
    #[error("recursive stream")]
    RecursiveStream,
    #[error("unused sequence type {0}")]
    UnusedSequence(Handle),
}

type Result<T> = result::Result<T, Error>;

#[derive(Clone, Copy)]
enum State {
    Unvisited,
    Visited,
    Validated,
}

#[derive(Clone, Copy)]
enum Status {
    Unvisited,
    Visited,
    Validated(BitWidth),
}

pub(super) struct Validator {
    states: Box<[State]>,
    pub bit_width: Box<[MaybeUninit<BitWidth>]>,
    pub referrers: Box<[Vec<Handle>]>,
    rec_sequences: Box<[bool]>,
}

impl Validator {
    pub(super) fn new(len: usize) -> Self {
        Self {
            states: vec![State::Unvisited; len].into(),
            bit_width: Box::new_uninit_slice(len),
            referrers: vec![vec![]; len].into(),
            rec_sequences: vec![false; len].into(),
        }
    }
    fn validated(&mut self, handle: Handle, bit_width: BitWidth) {
        self.states[handle.0] = State::Validated;
        self.bit_width[handle.0].write(bit_width);
    }
    fn status(&self, handle: Handle) -> Status {
        match self.states[handle.0] {
            State::Unvisited => Status::Unvisited,
            State::Visited => Status::Visited,
            State::Validated => {
                Status::Validated(unsafe { self.bit_width[handle.0].assume_init() })
            }
        }
    }
    pub(super) fn validate(&mut self, schema: &Schema) -> Result<()> {
        for (idx, def) in schema.definitions.iter().enumerate() {
            if !def.is_sequence() {
                def.validate(Handle(idx), self, schema)?;
            }
        }
        for (idx, state) in self.states.iter().enumerate() {
            if let State::Unvisited = state {
                Err(Error::UnusedSequence(Handle(idx)))?;
            }
        }
        for referers in &mut self.referrers {
            referers.dedup();
        }
        Ok(())
    }
}

impl Definition {
    fn is_sequence(&self) -> bool {
        matches!(self, Definition::Vector(_) | Definition::Stream(_))
    }
    fn validate(
        &self,
        handle: Handle,
        validator: &mut Validator,
        schema: &Schema,
    ) -> Result<BitWidth> {
        validator.states[handle.0] = State::Visited;
        let res = match self {
            &Definition::Primitive(Primitive { bit_width, .. }) => BitWidth::Fixed(bit_width as _),
            &Definition::Vector(Vector { elem_ty, .. })
            | &Definition::Stream(Stream { elem_ty, .. }) => {
                if validator.rec_sequences[handle.0] {
                    Err(Error::RecursiveStream)?;
                }
                validator.rec_sequences[handle.0] = true;
                let referrer = handle;
                if let Some((handle, inner)) = match elem_ty {
                    Type::Allocated(handle) => {
                        validator.referrers[handle.0].push(referrer);
                        let inner = schema.get_definition(handle)?;
                        match inner {
                            Definition::Vector(_) | Definition::Stream(_) => Some((handle, inner)),
                            _ => None,
                        }
                    }
                    _ => None,
                } {
                    inner.validate(handle, validator, schema)?;
                } else {
                    validator.rec_sequences.iter_mut().for_each(|x| *x = false);
                }
                BitWidth::Variable
            }
            Definition::Packed(Packed { members, .. }) => {
                let mut res = 0;
                for Member { ty, .. } in members {
                    match ty.validate(validator, schema, handle)? {
                        BitWidth::Fixed(x) => res += x,
                        BitWidth::Variable => Err(Error::RequiresFixedSize)?,
                    }
                }
                if res % 8 != 0 {
                    Err(Error::RequiresByteAligned)?
                }
                BitWidth::Fixed(res)
            }
            Definition::Struct(Struct { members, .. }) => {
                for &Member { ty, .. } in members {
                    if let BitWidth::Fixed(x) = ty.validate(validator, schema, handle)? {
                        if x % 8 != 0 {
                            Err(Error::RequiresByteAligned)?
                        }
                    }
                }
                BitWidth::Variable
            }
            &Definition::Enum(Enum {
                index_ty,
                ref members,
                ..
            }) => match &members[..] {
                &[] => BitWidth::Fixed(0),
                &[ref first, ref rest @ ..] => {
                    let index_size = index_ty as usize;
                    let index_bit_width = index_size * 8;
                    let index_cap = 1u64 << index_bit_width;

                    let mut used_indices = AHashSet::default();
                    if first.index >= index_cap {
                        Err(Error::IndexOutOfRange)?
                    }
                    used_indices.insert(first.index);

                    let mut member_bit_width =
                        first.member.ty.validate(validator, schema, handle)?;
                    for &IndexedMember {
                        index,
                        member: Member { ty, .. },
                    } in rest
                    {
                        if index >= index_cap {
                            Err(Error::IndexOutOfRange)?
                        }
                        if !used_indices.insert(index) {
                            Err(Error::DuplicateIndex)?
                        }
                        let bit_width = ty.validate(validator, schema, handle)?;
                        if let BitWidth::Fixed(x) = bit_width
                            && x % 8 != 0
                        {
                            Err(Error::RequiresByteAligned)?
                        }
                        if bit_width != member_bit_width {
                            member_bit_width = BitWidth::Variable
                        }
                    }
                    member_bit_width + index_bit_width
                }
            },
            &Definition::Dict(Dict {
                index_ty,
                ref members,
                ..
            }) => {
                let index_size = index_ty as usize;
                let index_bit_width = index_size * 8;
                let index_cap = index_bit_width as u64;

                let mut res = BitWidth::Fixed(index_bit_width);
                let mut used_indices = AHashSet::default();
                for &IndexedMember {
                    index,
                    member: Member { ty, .. },
                } in members
                {
                    if index >= index_cap {
                        Err(Error::IndexOutOfRange)?
                    }
                    if !used_indices.insert(index) {
                        Err(Error::DuplicateIndex)?
                    }
                    let bit_width = ty.validate(validator, schema, handle)?;
                    if let BitWidth::Fixed(x) = bit_width
                        && x % 8 != 0
                    {
                        Err(Error::RequiresByteAligned)?
                    }
                    if bit_width != BitWidth::Fixed(0) {
                        res = BitWidth::Variable
                    }
                }
                res
            }
            Definition::Extern(_) => BitWidth::Variable,
        };
        validator.validated(handle, res);
        Ok(res)
    }
}

impl Type {
    fn validate(
        &self,
        validator: &mut Validator,
        schema: &Schema,
        referrer: Handle,
    ) -> Result<BitWidth> {
        Ok(match self {
            &Type::Allocated(handle) => {
                let res = match validator.status(handle) {
                    Status::Unvisited => schema
                        .get_definition(handle)?
                        .validate(handle, validator, schema)?,
                    Status::Visited => Err(Error::RecursiveDefinition)?,
                    Status::Validated(x) => x,
                };
                validator.referrers[handle.0].push(referrer);
                res
            }
            &Type::Integer { bit_width, .. } => BitWidth::Fixed(bit_width as _),
            Type::Varint { .. } => BitWidth::Variable,
        })
    }
}

impl Schema {
    fn get_definition(&self, Handle(idx): Handle) -> Result<&Definition> {
        self.definitions.get(idx).ok_or(Error::HandleOutOfRange)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BitWidth {
    Fixed(usize),
    Variable,
}

impl BitWidth {
    pub fn fixed(self) -> Option<usize> {
        match self {
            BitWidth::Fixed(x) => Some(x),
            BitWidth::Variable => None,
        }
    }
}

impl Add for BitWidth {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        match (self, rhs) {
            (BitWidth::Fixed(x), BitWidth::Fixed(y)) => Self::Fixed(x + y),
            _ => Self::Variable,
        }
    }
}

impl Add<usize> for BitWidth {
    type Output = Self;

    fn add(self, rhs: usize) -> Self::Output {
        match (self, rhs) {
            (BitWidth::Fixed(x), y) => Self::Fixed(x + y),
            _ => Self::Variable,
        }
    }
}

impl<T> AddAssign<T> for BitWidth
where
    Self: Add<T, Output = Self>,
{
    fn add_assign(&mut self, rhs: T) {
        *self = *self + rhs
    }
}
