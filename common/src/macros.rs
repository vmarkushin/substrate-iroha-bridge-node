#[macro_export]
macro_rules! formula {
    ($int:literal,$dec:literal %) => {{
        // denominator = 10^(floor(log10(dec)) + 3)
        const DENOMINATOR: u32 = {
            let mut m = 100;
            let mut n = $dec;
            while n > 0 {
                n /= 10;
                m *= 10;
            }
            m
        };
        formula!($int %) + $crate::Fixed::from(($dec, DENOMINATOR))
    }};
    ($int:literal,$dec:literal) => {{
        // denominator = 10^(floor(log10(dec)) + 1)
        const DENOMINATOR: u32 = {
            let mut m = 1;
            let mut n = $dec;
            while n > 0 {
                n /= 10;
                m *= 10;
            }
            m
        };
        $crate::Fixed::from($int) + $crate::Fixed::from(($dec, DENOMINATOR))
    }};
    ($percent:literal %) => {
        $crate::Fixed::from(sp_arithmetic::Percent::from_parts($percent))
    };
    ($n:literal / $d:literal) => {
        $crate::Fixed::from(($n, $d))
    };
}

#[cfg(test)]
mod tests {
    use crate::Fixed;
    use sp_arithmetic::Percent;
    use sp_runtime::FixedPointNumber;

    #[test]
    #[rustfmt::skip]
    fn should_calculate_formula() {
        assert_eq!(formula!(1/2), Fixed::saturating_from_rational(1, 2));
        assert_eq!(formula!(10%), Fixed::from(Percent::from_parts(10)));
        assert_eq!(formula!(1,2), Fixed::saturating_from_rational(12, 10));
        assert_eq!(formula!(10,2), Fixed::saturating_from_rational(102, 10));
        assert_eq!(formula!(1,20), Fixed::saturating_from_rational(12, 10));
        assert_eq!(formula!(10,20), Fixed::saturating_from_rational(102, 10));
        assert_eq!(formula!(10,0), Fixed::from(10));
        assert_eq!(formula!(2,5%), Fixed::saturating_from_rational(25, 10_00));
        assert_eq!(formula!(20,5%), Fixed::saturating_from_rational(205, 10_00));
        assert_eq!(
            formula!(20,50%),
            Fixed::saturating_from_rational(205, 10_00)
        );
        assert_eq!(
            formula!(20,50%),
            Fixed::saturating_from_rational(205, 10_00)
        );
        assert_eq!(formula!(20,0%), Fixed::from(Percent::from_parts(20)));
    }
}
