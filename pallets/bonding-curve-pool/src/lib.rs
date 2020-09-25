#![cfg_attr(not(feature = "std"), no_std)]

#[macro_use]
extern crate alloc;

#[cfg(test)]
mod mock;

#[cfg(test)]
mod tests;

use codec::{Decode, Encode};
use common::prelude::{Balance, Fixed, FixedWrapper};
use common::{
    formula, AccountIdOf, AssetId, DEXId, LiquiditySource, LiquiditySourceId, LiquiditySourceType,
    SwapAmount, SwapOutcome,
};
use frame_support::sp_runtime::AccountId32;
use frame_support::traits::Get;
use frame_support::{decl_error, decl_module, decl_storage};
use orml_traits::MultiCurrency;
use sp_runtime::{DispatchError, FixedPointNumber, PerThing, Percent};

type TechAccountIdPrimitiveOf<T> = <T as technical::Trait>::TechAccountIdPrimitive;

pub trait Trait: common::Trait + assets::Trait + technical::Trait {
    type DEXApi: LiquiditySource<Self::DEXId, Self::AccountId, Self::AssetId, Fixed, DispatchError>;
}

#[derive(Debug, Encode, Decode)]
pub struct DistributionAccounts<T: Trait> {
    xor_allocation: T::TechAccountIdPrimitive,
    sora_citizens: T::TechAccountIdPrimitive,
    stores_and_shops: T::TechAccountIdPrimitive,
    parliament_and_development: T::TechAccountIdPrimitive,
    projects: T::TechAccountIdPrimitive,
}

impl<T: Trait> Default for DistributionAccounts<T> {
    fn default() -> Self {
        Self {
            xor_allocation: Default::default(),
            sora_citizens: Default::default(),
            stores_and_shops: Default::default(),
            parliament_and_development: Default::default(),
            projects: Default::default(),
        }
    }
}

decl_storage! {
    trait Store for Module<T: Trait> as BondingCurve {
        // TODO: technical
        ReservesAcc get(fn reserves_acc): T::TechAccountIdPrimitive;
        Reserves get(fn reserves): map hasher(twox_64_concat) T::DEXId => Fixed; // USD reserves
        InitialPrice get(fn initial_price) config(): Fixed = formula!(99,3);
        PriceChangeStep get(fn price_change_step) config(): Fixed = 5000.into();
        PriceChangeRate get(fn price_change_rate) config(): Fixed = 100.into();
        SellPriceCoefficient get(fn sell_price_coefficient) config(): Fixed = formula!(80%);
        DistributionAccountsEntry get(fn distribution_accounts) config(): DistributionAccounts<T>;
    }
}

decl_error! {
    pub enum Error for Module<T: Trait> {
        /// An error occurred while calculating the price.
        CalculatePriceFailed,
    }
}

decl_module! {
    pub struct Module<T: Trait> for enum Call where origin: T::Origin {
        type Error = Error<T>;
    }
}

impl<T: Trait> Module<T>
where
    AccountIdOf<T>: From<AccountId32>,
    AccountId32: From<AccountIdOf<T>>,
    TechAccountIdPrimitiveOf<T>: common::WrappedRepr<AccountId32>,
{
    /// Calculates and returns current buy price for one token.
    ///
    /// For every `PC_S` tokens the price goes up by `PC_R`.
    ///
    /// `P_B(Q) = Q / (PC_S * PC_R) + P_I`
    ///
    /// where
    /// `P_B(Q)`: buy price for one token
    /// `P_I`: initial token price
    /// `PC_R`: price change rate
    /// `PC_S`: price change step
    /// `Q`: token issuance (quantity)
    #[allow(non_snake_case)]
    pub fn buy_price(asset_id: T::AssetId) -> Result<Fixed, DispatchError> {
        let total_issuance_integer = assets::Module::<T>::total_issuance(&asset_id)?;
        let Q: FixedWrapper = total_issuance_integer.into();
        let P_I = Self::initial_price();
        let PC_S = Self::price_change_step();
        let PC_R = Self::price_change_rate();
        let price = Q / (PC_S * PC_R) + P_I;
        price.get().ok_or(Error::<T>::CalculatePriceFailed.into())
    }

    /// Calculates and returns current buy price for some amount of token.
    ///
    /// To calculate _buy_ price for a specific amount of tokens,
    /// one needs to integrate the equation of buy price (`P_B(Q)`):
    ///
    /// ```nocompile
    /// P_BTI(Q, q) = integrate [P_B(x) dx, x = Q to Q+q]
    ///            = integrate [x / (PC_S * PC_R) + P_I dx, x = Q to Q+q]
    ///            = x^2 / (2 * PC_S * PC_R) + P_I * x, x = Q to Q+q
    ///            = (x / (2 * PC_S * PC_R) + P_I) * x
    ///            = ((Q+q) / (2 * PC_S * PC_R) + P_I) * (Q+q) -
    ///              (( Q ) / (2 * PC_S * PC_R) + P_I) * ( Q )
    /// ```
    /// where
    /// `P_BTI(Q, q)`: buy price for `q` tokens
    /// `Q`: current token issuance (quantity)
    /// `q`: amount of tokens to buy
    ///
    /// P = 2 * x
    /// $20 = 2 * 10 (for 1 XOR)
    ///
    #[allow(non_snake_case)]
    #[rustfmt::skip]
    pub fn buy_in_tokens_price(asset_id: T::AssetId, quantity: Balance) -> Result<Fixed, DispatchError> {
        let total_issuance_integer = assets::Module::<T>::total_issuance(&asset_id)?;
        let Q = FixedWrapper::from(total_issuance_integer);
        let P_I = Self::initial_price();
        let PC_S = FixedWrapper::from(Self::price_change_step());
        let PC_R = Self::price_change_rate();

        let Q_plus_q = Q + quantity;
        let two_times_PC_S_times_PC_R = 2 * PC_S * PC_R;
        let to   = (Q_plus_q / two_times_PC_S_times_PC_R + P_I) * Q_plus_q;
        let from = (Q        / two_times_PC_S_times_PC_R + P_I) * Q;
        let price: FixedWrapper = to - from;
        price.get().ok_or(Error::<T>::CalculatePriceFailed.into())
    }

    /// P_BTO = P_BTI^(-1)
    ///     //  = PC_S * PC_R * P_I +- sqrt(PC_S * PC_R * (PC_S * PC_R * P_I^2 + 2 * x))
    ///       =  ± PC *
    ///         sqrt((Q^2 + 2 * Q * PC * P_I + PC^2 * P_I^2 + 2 * PC * x) / PC^2)
    ///         -Q - PC * P_I
    ///
    /// where
    /// PC = PC_S * PC_R
    /// [calc](https://www.wolframalpha.com/input/?i=y+%3D+%28%28a%2Bx%29+%2F+%282+*+b+*+c%29+%2B+d%29+*+%28a%2Bx%29+-+%28+a+%2F+%282+*+b+*+c%29+%2B+d%29+*+a+inv)
    #[allow(non_snake_case)]
    #[rustfmt::skip]
    pub fn buy_out_tokens_price(asset_id: T::AssetId, quantity: Balance) -> Result<Fixed, DispatchError> {
        let total_issuance_integer = assets::Module::<T>::total_issuance(&asset_id)?;
        let Q = FixedWrapper::from(total_issuance_integer);
        let P_I = Self::initial_price();
        let PC_S = FixedWrapper::from(Self::price_change_step());
        let PC_R = Self::price_change_rate();

        let q = quantity;
        let PC = PC_S * PC_R;
        let PC_sqr = PC * PC;
        let PC_times_P_I = PC * P_I;
        let price = PC * ((Q*Q + 2 * Q * PC * P_I + PC_sqr * P_I * P_I + 2 * PC * q) / PC_sqr).sqrt();
        todo!()
    }

    /// Calculates and returns current sell price for one token.
    /// Sell price is `P_Sc`% of buy price (see `buy_price`).
    ///
    /// `P_S = P_Sc * P_B`
    /// where
    /// `P_Sc: sell price coefficient (%)`
    #[allow(non_snake_case)]
    pub fn sell_price(asset_id: T::AssetId) -> Result<Fixed, DispatchError> {
        let P_B = Self::buy_price(asset_id)?;
        let P_Sc = FixedWrapper::from(Self::sell_price_coefficient());
        let price = P_Sc * P_B;
        price.get().ok_or(Error::<T>::CalculatePriceFailed.into())
    }

    /// Calculates and returns current sell price for some amount of token.
    /// Sell tokens price is `P_Sc`% of buy tokens price (see `buy_tokens_price`).
    ///
    /// ```nocompile
    /// P_ST = integrate [P_S dx]
    ///      = integrate [P_Sc * P_B dx]
    ///      = P_Sc * integrate [P_B dx]
    ///      = P_Sc * P_BT
    /// where
    /// `P_Sc: sell price coefficient (%)`
    /// ```
    #[allow(non_snake_case)]
    pub fn sell_tokens_price(
        asset_id: T::AssetId,
        quantity: Balance,
    ) -> Result<Fixed, DispatchError> {
        let P_BT = Self::buy_in_tokens_price(asset_id, quantity)?;
        let P_Sc = FixedWrapper::from(Self::sell_price_coefficient());
        let price = P_Sc * P_BT;
        price.get().ok_or(Error::<T>::CalculatePriceFailed.into())
    }

    #[allow(non_snake_case)]
    pub fn buy(
        dex_id: T::DEXId,
        asset_id: T::AssetId,
        for_amount: Balance,
        from_account_id: T::AccountId,
    ) -> Result<(), DispatchError> {
        let mut R = Self::reserves(&dex_id);
        let total_issuance_integer = assets::Module::<T>::total_issuance(&asset_id)?;
        let R_expected = Self::sell_tokens_price(asset_id, total_issuance_integer)?;
        if R < R_expected {
            R = R + for_amount.0;
        }
        if R > R_expected {
            let reserves_free_coefficient: Fixed = formula!(20%);
            let R_free = reserves_free_coefficient * (R - R_expected);
            let val_holders_coefficient: Fixed = formula!(50%);
            let val_holders_xor_alloc_coeff = val_holders_coefficient * formula!(90%);
            let val_holders_buy_back_coefficient =
                val_holders_coefficient * (formula!(100%) - val_holders_xor_alloc_coeff); // TODO
            let projects_coefficient = formula!(100%) - val_holders_coefficient;
            let projects_sora_citizens_coeff = projects_coefficient * formula!(1%);
            let projects_stores_and_shops_coeff = projects_coefficient * formula!(4%);
            let projects_parliament_and_development_coeff = projects_coefficient * formula!(5%);
            let projects_other_coeff = projects_coefficient * formula!(90%);
            let dist_accounts: DistributionAccounts<T> = Self::distribution_accounts();

            #[rustfmt::skip]
            let distributions = vec![
                (dist_accounts.xor_allocation, val_holders_xor_alloc_coeff),
                (dist_accounts.projects, projects_other_coeff),
                (dist_accounts.sora_citizens, projects_sora_citizens_coeff),
                (dist_accounts.stores_and_shops, projects_stores_and_shops_coeff),
                (dist_accounts.parliament_and_development, projects_parliament_and_development_coeff),
            ];
            for (to_account, coefficient) in distributions {
                let to_tech_account =
                    technical::Module::<T>::tech_acc_id_from_primitive(to_account);
                technical::Module::<T>::set_transfer_in(
                    asset_id,
                    from_account_id.clone(),
                    to_tech_account,
                    Balance(R_free * coefficient),
                )?;
            }
            let reserves_acc = {
                let opt: Option<_> = Self::reserves_acc().into();
                opt
            }
            .ok_or(Error::<T>::CalculatePriceFailed)?;
            T::DEXApi::exchange(
                &reserves_acc,
                &reserves_acc,
                &DEXId::Polkaswap.into(),
                &AssetId::XOR.into(),
                &AssetId::VAL.into(),
                SwapAmount::with_desired_input(
                    R_free * val_holders_buy_back_coefficient,
                    Fixed::zero(),
                ),
            );
            R = R - R_free;
        }
        Reserves::<T>::mutate(&dex_id, |balance| {
            *balance = R;
        });
        Ok(())
    }
}

impl<T: Trait> LiquiditySource<T::DEXId, T::AccountId, T::AssetId, Fixed, DispatchError>
    for Module<T>
{
    fn can_exchange(
        dex_id: &T::DEXId,
        input_asset_id: &T::AssetId,
        output_asset_id: &T::AssetId,
    ) -> bool {
        let base_asset_id = &T::GetBaseAssetId::get();
        // can trade only XOR (base asset) <-> USD on Polkaswap
        *dex_id == DEXId::Polkaswap.into()
            && ((input_asset_id == &AssetId::USD.into() && output_asset_id == base_asset_id)
                || (output_asset_id == &AssetId::USD.into() && input_asset_id == base_asset_id))
    }

    fn quote(
        dex_id: &T::DEXId,
        input_asset_id: &T::AssetId,
        output_asset_id: &T::AssetId,
        swap_amount: SwapAmount<Fixed>,
    ) -> Result<SwapOutcome<Fixed>, DispatchError> {
        if !Self::can_exchange(dex_id, input_asset_id, output_asset_id) {
            todo!("return error");
        }
        let base_asset_id = &T::GetBaseAssetId::get();
        if input_asset_id == base_asset_id {
            match swap_amount {
                SwapAmount::WithDesiredInput {
                    desired_amount_in: base_amount_in,
                    ..
                } => todo!(),
                SwapAmount::WithDesiredOutput {
                    desired_amount_out: target_amount_out,
                    ..
                } => todo!(),
            }
        } else {
            match swap_amount {
                SwapAmount::WithDesiredInput {
                    desired_amount_in: target_amount_in,
                    ..
                } => todo!(),
                SwapAmount::WithDesiredOutput {
                    desired_amount_out: base_amount_in,
                    ..
                } => todo!(),
            }
        }

        /*
        let base_asset_id = &T::GetBaseAssetId::get();
        if input_asset_id == base_asset_id {
            let (base_reserve, target_reserve) = <Reserves<T>>::get(dex_id, output_asset_id);
            match swap_amount {
                SwapAmount::WithDesiredInput {
                    desired_amount_in: base_amount_in,
                    ..
                } => Ok(Self::get_target_amount_out(
                    base_amount_in,
                    base_reserve,
                    target_reserve,
                )?),
                SwapAmount::WithDesiredOutput {
                    desired_amount_out: target_amount_out,
                    ..
                } => Ok(Self::get_base_amount_in(
                    target_amount_out,
                    base_reserve,
                    target_reserve,
                )?),
            }
        } else if output_asset_id == base_asset_id {
            let (base_reserve, target_reserve) = <Reserves<T>>::get(dex_id, input_asset_id);
            match swap_amount {
                SwapAmount::WithDesiredInput {
                    desired_amount_in: target_amount_in,
                    ..
                } => Ok(Self::get_base_amount_out(
                    target_amount_in,
                    base_reserve,
                    target_reserve,
                )?),
                SwapAmount::WithDesiredOutput {
                    desired_amount_out: base_amount_out,
                    ..
                } => Ok(Self::get_target_amount_in(
                    base_amount_out,
                    base_reserve,
                    target_reserve,
                )?),
            }
        } else {
            let (base_reserve_a, target_reserve_a) = <Reserves<T>>::get(dex_id, input_asset_id);
            let (base_reserve_b, target_reserve_b) = <Reserves<T>>::get(dex_id, output_asset_id);
            match swap_amount {
                SwapAmount::WithDesiredInput {
                    desired_amount_in, ..
                } => {
                    let outcome_a = Self::get_base_amount_out(
                        desired_amount_in,
                        base_reserve_a,
                        target_reserve_a,
                    )?;
                    let outcome_b = Self::get_target_amount_out(
                        outcome_a.amount,
                        base_reserve_b,
                        target_reserve_b,
                    )?;
                    Ok(SwapOutcome::new(
                        outcome_b.amount,
                        outcome_a.fee + outcome_b.fee,
                    ))
                }
                SwapAmount::WithDesiredOutput {
                    desired_amount_out, ..
                } => {
                    let outcome_b = Self::get_base_amount_in(
                        desired_amount_out,
                        base_reserve_b,
                        target_reserve_b,
                    )?;
                    let outcome_a = Self::get_target_amount_in(
                        outcome_b.amount,
                        base_reserve_a,
                        target_reserve_a,
                    )?;
                    Ok(SwapOutcome::new(
                        outcome_a.amount,
                        outcome_b.fee + outcome_a.fee,
                    ))
                }
            }
        }
         */
        unimplemented!()
    }

    fn exchange(
        sender: &T::AccountId,
        receiver: &T::AccountId,
        dex_id: &T::DEXId,
        input_asset_id: &T::AssetId,
        output_asset_id: &T::AssetId,
        desired_amount: SwapAmount<Fixed>,
    ) -> Result<SwapOutcome<Fixed>, DispatchError> {
        let reserves_acc = &Self::reserves_acc()
            .into()
            .ok_or(Error::<T>::CalculatePriceFailed)?;
        if sender == reserves_acc && receiver == reserves_acc {
            todo!("return error?");
        }
        Self::quote(dex_id, input_asset_id, output_asset_id, desired_amount)
    }
}
