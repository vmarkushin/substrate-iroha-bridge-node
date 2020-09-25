mod tests {
    use crate::{mock::*, Error};
    use common::prelude::Fixed;

    #[test]
    fn should_calculate_price() {
        let mut ext = ExtBuilder::default().build();
        ext.execute_with(|| {
            assert_eq!(
                BondingCurvePool::buy_price(XOR).expect("failed to calculate buy price"),
                Fixed::from(100)
            );
            assert_eq!(
                BondingCurvePool::buy_in_tokens_price(XOR, 100_000u32.into())
                    .expect("failed to calculate buy tokens price"),
                Fixed::from(100_10_000)
            );
            assert_eq!(
                BondingCurvePool::sell_price(XOR).expect("failed to calculate sell price"),
                Fixed::from(80)
            );
            assert_eq!(
                BondingCurvePool::sell_tokens_price(XOR, 100_000u32.into())
                    .expect("failed to calculate sell tokens price"),
                Fixed::from(80_08_000)
            );
            assert_eq!(
                BondingCurvePool::sell_tokens_price(XOR, 0u32.into())
                    .expect("failed to calculate sell tokens price"),
                Fixed::from(0)
            );
        });
    }

    #[test]
    fn should_not_calculate_price() {
        let mut ext = ExtBuilder::default().build();
        ext.execute_with(|| {
            assert_eq!(
                BondingCurvePool::sell_tokens_price(XOR, u128::max_value().into()).unwrap_err(),
                Error::<Runtime>::CalculatePriceFailed.into()
            );
        });
    }
}
