use thiserror::Error;

use crate::byte_stream::{ByteStreamLe, ReadBitfieldError};

#[derive(Error, Debug)]
#[error("one or more bits were set but never checked: {0:#x} ({0:#b})")]
pub struct UnhandledBitsError(u32);

/// Wraps a 32-bit bitfield and tracks which bits have been queried.
///
/// If we read a bitfield from a binary file, we need to handle all of the bits that are used;
/// if a bit is set but we never check its value, it may lead to parsing errors later that are
/// hard to diagnose.
#[derive(Default, Clone, Copy, Debug)]
pub struct CheckedBitfield {
    /// The underlying bitfield.
    bits: u32,

    /// Stores which bits of `bits` have been queried.
    checked: u32,
}

impl<I: Into<u32>> From<I> for CheckedBitfield {
    fn from(value: I) -> CheckedBitfield {
        CheckedBitfield {
            bits: value.into(),
            checked: 0,
        }
    }
}

impl CheckedBitfield {
    pub fn try_parse(stream: &mut impl ByteStreamLe) -> Result<CheckedBitfield, ReadBitfieldError> {
        stream.read_variable_length_bitfield().map(From::from)
    }

    pub const fn clear(&mut self) {
        self.bits = 0;
        self.checked = 0;
    }

    /// Returns `true` iff the `index`th bit is set, where 0 is the index of the least significant
    /// bit. Stores that this bit has been checked.
    pub const fn check_bit(&mut self, index: u8) -> bool {
        let mask = 1 << index;
        self.checked |= mask;
        self.bits & mask != 0
    }

    /// Returns `true` iff any bits are set.
    pub const fn any_set(self) -> bool {
        self.bits != 0
    }

    /// Returns an `UnhandledBitsError` containing the unhandled bits if there are any.
    pub const fn ensure_none_set_unchecked(self) -> Result<(), UnhandledBitsError> {
        // Match on the bits that are set in `bits` but not in `checked`.
        match self.bits & !self.checked {
            0 => Ok(()),
            bad => Err(UnhandledBitsError(bad)),
        }
    }
}

#[macro_export]
macro_rules! option_on_bit {
    ($bf:expr, $i:expr => $then:expr $(,)?) => {
        if $bf.check_bit($i) { Some($then) } else { None }
    };

    ($bf:expr, $i:expr => $then:expr, else $default:expr $(,)?) => {
        if $bf.check_bit($i) { $then } else { $default }
    };
}

#[macro_export]
macro_rules! unpack_field_flags {
    ($bf:expr, {$($i:literal => $name:ident: $then:expr $(, else $default:expr)?;)+}) => {
        $(
            let $name = $crate::option_on_bit!($bf, $i => $then $(, else $default)?);
        )*
    };
}

#[macro_export]
macro_rules! unpack_bool_flag {
    // Typical case: true iff the bit is set
    ($bf:expr, $i:literal => $name:ident) => {
        let $name = $bf.check_bit($i);
    };

    // Negated case: true iff the bit is not set
    ($bf:expr, $i:literal => !$name:ident) => {
        let $name = !$bf.check_bit($i);
    };
}

#[macro_export]
macro_rules! unpack_bool_flags {
    ($bf:expr, {$($i:literal => $tx:tt $($ty:ident)?;)+}) => {
        $(
            $crate::unpack_bool_flag!($bf, $i => $tx $($ty)?);
        )*
    };
}

#[macro_export]
macro_rules! read_flags {
    ($stream:expr, $bf_name:ident, $parse_err:expr, $unhandled_err:expr, { $($do:tt)* }) => {
        let mut $bf_name = CheckedBitfield::try_parse($stream).map_err($parse_err)?;

        $($do)*

        $bf_name.ensure_none_set_unchecked().map_err($unhandled_err)?;
    };
}

#[macro_export]
macro_rules! impl_try_from_for_optional_from {
    ($target:ty, $prim:ty, $fromfn:ident, $v:vis $errtype:ident) => {
        #[derive(thiserror::Error, Debug)]
        #[error("invalid value {bad_value} for {}", stringify!($target))]
        $v struct $errtype {
            bad_value: $prim,
        }

        impl TryFrom<$prim> for $target {
            type Error = $errtype;

            fn try_from(v: $prim) -> Result<$target, $errtype> {
                <$target>::$fromfn(v).ok_or($errtype {
                    bad_value: v,
                })
            }
        }
    };
}
