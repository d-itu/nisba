#![no_std]

#[cfg(feature = "alloc")]
extern crate alloc;

mod private {
    pub trait Sealed {}
}

pub mod wrappers {
    use crate::private::Sealed;
    #[repr(transparent)]
    pub struct Integer<const N: usize>(pub(crate) [u8; N]);
    impl<const N: usize> Sealed for Integer<N> {}

    #[repr(transparent)]
    pub struct Primitive<T>(pub(crate) T);
    impl<T> Sealed for Primitive<T> {}

    #[repr(transparent)]
    pub struct Varint<T>(pub(crate) T);
    impl<T> Sealed for Varint<T> {}
}

#[macro_export]
macro_rules! const_try {
    ($x:expr) => {
        match $x {
            Ok(x) => x,
            Err(e) => return Err(e),
        }
    };
}

pub use static_assertions::const_assert_eq;

pub mod decode;
pub mod encode;
