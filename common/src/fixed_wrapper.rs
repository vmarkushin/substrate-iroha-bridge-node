use crate::balance::Balance;
use crate::Fixed;
use core::ops::*;
use frame_support::sp_runtime::traits::{CheckedAdd, CheckedDiv, CheckedMul, CheckedSub};
use sp_arithmetic::FixedPointNumber;
use static_assertions::_core::cmp::Ordering;

/// A convenient wrapper around `Fixed` type for safe math.
///
/// Supported operations: `+`, '-', '/', '*'.
#[derive(Clone, Copy)]
pub struct FixedWrapper {
    inner: Option<Fixed>,
}

impl FixedWrapper {
    /// Retrieve the result.
    ///
    /// If returned value is `None`, then an error were occurred during calculation.
    pub fn get(self) -> Option<Fixed> {
        self.inner
    }

    /// Newton-Raphson Method
    fn sqrt2() {
        unimplemented!()
    }

    ///
    pub fn sqrt(self) -> Self {
        Self::from(self.to_fraction().map(|x| Self::from_fraction(x.sqrt())))
    }

    pub fn from_fraction(x: f64) -> Fixed {
        Fixed::from_inner(
            (x * (<Fixed as FixedPointNumber>::DIV as f64)) as <Fixed as FixedPointNumber>::Inner,
        )
    }

    pub fn to_fraction(&self) -> Option<f64> {
        self.inner
            .map(|x| x.into_inner() as f64 / <Fixed as FixedPointNumber>::DIV as f64)
    }
}

impl From<Option<Fixed>> for FixedWrapper {
    fn from(option: Option<Fixed>) -> Self {
        FixedWrapper { inner: option }
    }
}

impl From<Fixed> for FixedWrapper {
    fn from(fixed: Fixed) -> Self {
        FixedWrapper::from(Some(fixed))
    }
}

impl From<Balance> for FixedWrapper {
    fn from(balance: Balance) -> Self {
        FixedWrapper::from(balance.0)
    }
}

impl From<u128> for FixedWrapper {
    fn from(int: u128) -> Self {
        FixedWrapper::from(Fixed::from(int))
    }
}

macro_rules! impl_op_for_fixed_wrapper {
    (
        $op:ty,
        $op_fn:ident,
        $checked_op_fn:ident
    ) => {
        impl $op for FixedWrapper {
            type Output = Self;

            fn $op_fn(self, rhs: Self) -> Self::Output {
                let lhs = self.inner;
                let rhs = rhs.inner;
                lhs.zip(rhs)
                    .and_then(|(lhs, rhs)| lhs.$checked_op_fn(&rhs))
                    .into()
            }
        }
    };
}

impl_op_for_fixed_wrapper!(Add, add, checked_add);
impl_op_for_fixed_wrapper!(Sub, sub, checked_sub);
impl_op_for_fixed_wrapper!(Mul, mul, checked_mul);
impl_op_for_fixed_wrapper!(Div, div, checked_div);

impl PartialEq for FixedWrapper {
    fn eq(&self, other: &Self) -> bool {
        let lhs = self.inner;
        let rhs = other.inner;
        lhs.zip(rhs).map(|(lhs, rhs)| lhs.eq(&rhs)).unwrap_or(false)
    }
}

impl PartialOrd for FixedWrapper {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        let lhs = self.inner;
        let rhs = other.inner;
        lhs.zip(rhs).and_then(|(lhs, rhs)| lhs.partial_cmp(&rhs))
    }
}

macro_rules! impl_op_fixed_wrapper_for_type {
    (
        $op:ident,
        $op_fn:ident,
        $type:ty
    ) => {
        // left (FixedWrapper + $type)
        impl $op<$type> for FixedWrapper {
            type Output = Self;

            fn $op_fn(self, rhs: $type) -> Self::Output {
                if self.inner.is_none() {
                    return None.into();
                }
                let rhs = FixedWrapper::from(rhs);
                self.$op_fn(rhs)
            }
        }
        // right ($type + FixedWrapper)
        impl $op<FixedWrapper> for $type {
            type Output = FixedWrapper;

            fn $op_fn(self, rhs: FixedWrapper) -> Self::Output {
                if rhs.inner.is_none() {
                    return None.into();
                }
                let lhs = FixedWrapper::from(self);
                lhs.$op_fn(rhs)
            }
        }
    };
}

macro_rules! impl_fixed_wrapper_for_type {
    ($type:ty) => {
        impl_op_fixed_wrapper_for_type!(Add, add, $type);
        impl_op_fixed_wrapper_for_type!(Sub, sub, $type);
        impl_op_fixed_wrapper_for_type!(Mul, mul, $type);
        impl_op_fixed_wrapper_for_type!(Div, div, $type);
    };
}

// Here one can add more custom implementations.
impl_fixed_wrapper_for_type!(Fixed);
impl_fixed_wrapper_for_type!(Balance);
impl_fixed_wrapper_for_type!(u128);
