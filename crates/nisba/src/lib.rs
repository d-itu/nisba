#![no_std]

#[cfg(feature = "alloc")]
extern crate alloc;

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

#[doc(hidden)]
pub use nisba_macros as __macros;

#[macro_export]
macro_rules! generate_encode {
    ($s:literal) => {
        $crate::__macros::generate_encode!($s);
    };
}

#[macro_export]
macro_rules! generate_decode {
    ($s:literal) => {
        $crate::__macros::generate_decode!($s);
    };
}

pub mod decode;
pub mod encode;
