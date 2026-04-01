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

#[derive(Clone, Copy)]
enum Constraint {
    None,
    FixedSize {
        #[allow(dead_code)]
        by: Handle,
    },
}

pub(super) struct Validator {
    states: Box<[State]>,
    pub bit_width: Box<[MaybeUninit<BitWidth>]>,
    pub has_lifetime: Box<[bool]>,
    constraints: Box<[Constraint]>,
    rec_streams: Box<[bool]>,
}

impl Validator {
    pub(super) fn new(len: usize) -> Self {
        Self {
            states: vec![State::Unvisited; len].into(),
            bit_width: Box::new_uninit_slice(len),
            has_lifetime: vec![false; len].into(),
            constraints: vec![Constraint::None; len].into(),
            rec_streams: vec![false; len].into(),
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
            &Definition::Vector(Vector { elem_ty, .. }) => {
                validator.has_lifetime[handle.0] = true;
                match elem_ty {
                    Type::Allocated(Handle(idx)) => {
                        if schema.get_definition(Handle(idx))?.is_sequence() {
                            Err(Error::RequiresByteAligned)?
                        } else {
                            validator.constraints[idx] = Constraint::FixedSize { by: handle }
                        }
                    }
                    Type::Integer { bit_width, .. } => {
                        if bit_width % 8 != 0 {
                            Err(Error::RequiresByteAligned)?
                        }
                    }
                    Type::Varint { .. } => Err(Error::RequiresFixedSize)?,
                }
                BitWidth::Variable
            }
            &Definition::Stream(Stream { elem_ty, .. }) => {
                validator.has_lifetime[handle.0] = true;
                if validator.rec_streams[handle.0] {
                    Err(Error::RecursiveStream)?;
                }
                let inner = match elem_ty {
                    Type::Allocated(handle) => match schema.get_definition(handle)? {
                        def @ Definition::Vector(_) => {
                            def.validate(handle, validator, schema)?;
                            None
                        }
                        def @ Definition::Stream(_) => {
                            def.validate(handle, validator, schema)?;
                            Some(handle.0)
                        }
                        _ => None,
                    },
                    _ => None,
                };
                match inner {
                    Some(x) => validator.rec_streams[x] = true,
                    None => validator.rec_streams.iter_mut().for_each(|x| *x = false),
                }
                BitWidth::Variable
            }
            Definition::Packed(Packed { members, .. }) => {
                let mut res = 0;
                for Member { ty, .. } in members {
                    match ty.bit_width(validator, schema)? {
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
                    if let BitWidth::Fixed(x) = ty.bit_width(validator, schema)? {
                        if x % 8 != 0 {
                            Err(Error::RequiresByteAligned)?
                        }
                    }
                    if let Type::Allocated(x) = ty {
                        validator.has_lifetime[handle.0] |= validator.has_lifetime[x.0]
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

                    let mut member_bit_width = first.member.ty.bit_width(validator, schema)?;
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
                        let bit_width = ty.bit_width(validator, schema)?;
                        if let BitWidth::Fixed(x) = bit_width
                            && x % 8 != 0
                        {
                            Err(Error::RequiresByteAligned)?
                        }
                        if let Type::Allocated(x) = ty {
                            validator.has_lifetime[handle.0] |= validator.has_lifetime[x.0]
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
                    let bit_width = ty.bit_width(validator, schema)?;
                    if let BitWidth::Fixed(x) = bit_width
                        && x % 8 != 0
                    {
                        Err(Error::RequiresByteAligned)?
                    }
                    if let Type::Allocated(x) = ty {
                        validator.has_lifetime[handle.0] |= validator.has_lifetime[x.0]
                    }
                    if bit_width != BitWidth::Fixed(0) {
                        res = BitWidth::Variable
                    }
                }
                res
            }
        };
        match validator.constraints[handle.0] {
            Constraint::None => {}
            Constraint::FixedSize { .. } => res.validate_fixed()?,
        }
        validator.validated(handle, res);
        Ok(res)
    }
}

impl Type {
    fn bit_width(&self, validator: &mut Validator, schema: &Schema) -> Result<BitWidth> {
        Ok(match self {
            &Type::Allocated(handle) => match validator.status(handle) {
                Status::Unvisited => schema
                    .get_definition(handle)?
                    .validate(handle, validator, schema)?,
                Status::Visited => Err(Error::RecursiveDefinition)?,
                Status::Validated(x) => x,
            },
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
    fn validate_fixed(self) -> Result<()> {
        match self {
            BitWidth::Fixed(_) => Ok(()),
            BitWidth::Variable => Err(Error::RequiresFixedSize),
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
