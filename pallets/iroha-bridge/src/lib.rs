//! A demonstration of an offchain worker that sends onchain callbacks

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(test)]
mod tests;

#[cfg(test)]
pub mod mock;

#[macro_use]
mod utils;

use alt_serde::{Deserialize, Deserializer};
use core::{convert::TryInto, fmt};
use core::{line, stringify};
use frame_support::dispatch::Weight;
use frame_support::traits::Currency;
use frame_support::traits::ExistenceRequirement;
use frame_support::{
    debug, decl_error, decl_event, decl_module, decl_storage, dispatch::DispatchResult, traits::Get,
};
use frame_system::offchain::{Account, SignMessage, SigningTypes};
use frame_system::RawOrigin;
use frame_system::{
    self as system, ensure_none, ensure_root, ensure_signed,
    offchain::{
        AppCrypto, CreateSignedTransaction, SendSignedTransaction, Signer, SubmitTransaction,
    },
};
use iroha_client_no_std::account;
use iroha_client_no_std::account::isi::AccountInstruction;
use iroha_client_no_std::account::query::GetAccount;
use iroha_client_no_std::asset::isi::AssetInstruction;
use iroha_client_no_std::asset::query::GetAccountAssets;
use iroha_client_no_std::block::{BlockHeader, Message as BlockMessage, Message, ValidBlock};
use iroha_client_no_std::bridge;
use iroha_client_no_std::bridge::asset::ExternalAsset;
use iroha_client_no_std::bridge::{BridgeDefinitionId, ExternalTransaction};
use iroha_client_no_std::crypto as iroha_crypto;
use iroha_client_no_std::isi::prelude::PeerInstruction;
use iroha_client_no_std::peer::PeerId;
use iroha_client_no_std::prelude as iroha;
use iroha_client_no_std::tx::{Payload, RequestedTransaction};
use parity_scale_codec::{Decode, Encode};
use sp_core::crypto::KeyTypeId;
use sp_core::ed25519::Signature as SpSignature;
use sp_core::{crypto::AccountId32, ed25519, sr25519};
use sp_runtime::offchain::http::Request;
use sp_runtime::traits::{Hash, StaticLookup};
use sp_runtime::traits::{IdentifyAccount, Verify};
use sp_runtime::DispatchError;
use sp_runtime::{
    offchain as rt_offchain,
    offchain::storage::StorageValueRef,
    transaction_validity::{
        InvalidTransaction, TransactionPriority, TransactionSource, TransactionValidity,
        ValidTransaction,
    },
    MultiSignature,
};
use sp_std::convert::TryFrom;
use sp_std::prelude::*;
use sp_std::str;
use treasury::AssetKind;

pub const KEY_TYPE: KeyTypeId = KeyTypeId(*b"demo");
pub const KEY_TYPE_2: KeyTypeId = KeyTypeId(*b"dem0");
pub const NUM_VEC_LEN: usize = 10;

pub const INSTRUCTION_ENDPOINT: &str = "http://127.0.0.1:7878/instruction";
pub const BLOCK_ENDPOINT: &str = "http://127.0.0.1:7878/block";
pub const QUERY_ENDPOINT: &str = "http://127.0.0.1:7878/query";

pub mod crypto {
    use crate::KEY_TYPE;
    use sp_core::ecdsa::Signature as EcdsaSignature;
    use sp_core::ed25519::{Public as EdPublic, Signature as Ed25519Signature};
    use sp_core::sr25519::Signature as Sr25519Signature;

    use sp_runtime::{
        app_crypto::{app_crypto, ecdsa, ed25519, sr25519},
        traits::Verify,
        MultiSignature, MultiSigner,
    };

    app_crypto!(sr25519, KEY_TYPE);

    pub struct TestAuthId;

    // implemented for ocw-runtime
    impl frame_system::offchain::AppCrypto<MultiSigner, MultiSignature> for TestAuthId {
        type RuntimeAppPublic = Public;
        type GenericSignature = sp_core::sr25519::Signature;
        type GenericPublic = sp_core::sr25519::Public;
    }
}

pub mod crypto_ed {
    use crate::KEY_TYPE_2 as KEY_TYPE;
    use sp_core::ed25519::{Public as EdPublic, Signature as Ed25519Signature};

    use sp_runtime::{
        app_crypto::{app_crypto, ed25519},
        traits::Verify,
        MultiSignature, MultiSigner,
    };

    app_crypto!(ed25519, KEY_TYPE);

    pub struct TestAuthId;
    impl frame_system::offchain::AppCrypto<MultiSigner, MultiSignature> for TestAuthId {
        type RuntimeAppPublic = Public;
        type GenericSignature = sp_core::ed25519::Signature;
        type GenericPublic = sp_core::ed25519::Public;
    }
}

/// This is the pallet's configuration trait
pub trait Trait: system::Trait + treasury::Trait + CreateSignedTransaction<Call<Self>> {
    /// The identifier type for an offchain worker.
    type AuthorityId: AppCrypto<Self::Public, Self::Signature>;
    /// The identifier type for an offchain worker with Ed25519 keys.
    type AuthorityIdEd: AppCrypto<Self::Public, Self::Signature>;
    /// The overarching dispatch call type.
    type Call: From<Call<Self>>;
    /// The overarching event type.
    type Event: From<Event<Self>> + Into<<Self as system::Trait>::Event>;
    /// The type to sign and send transactions.
    type UnsignedPriority: Get<TransactionPriority>;
}

/// The type of requests we can send to the offchain worker
#[cfg_attr(feature = "std", derive(PartialEq, Eq, Debug))]
#[derive(Encode, Decode)]
pub enum OffchainRequest<T: system::Trait + treasury::Trait> {
    /// Outgoing transfer from Substrate to Iroha request.
    OutgoingTransfer(
        T::AccountId,
        iroha::AccountId,
        treasury::AssetKind,
        u128,
        u8,
    ),
}

decl_storage! {
    trait Store for Module<T: Trait> as Example {
        /// Requests for off-chain workers made within this block execution
        OcRequests get(fn oc_requests): Vec<OffchainRequest<T>>;
        Authorities get(fn authorities) config(): Vec<T::AccountId>;
        Accounts: map hasher(twox_64_concat) iroha::AccountId => T::AccountId;
    }
}

decl_event!(
    /// Events generated by the module.
    pub enum Event<T>
    where
        AccId = <T as system::Trait>::AccountId,
    {
        IncomingTransfer(iroha::AccountId, AccId, AssetKind, u128),
        OutgoingTransfer(AccId, iroha::AccountId, AssetKind, u128),
    }
);

decl_error! {
    pub enum Error for Module<T: Trait> {
        HttpFetchingError,
        AlreadyFetched,
        ReserveCollateralError,
        AccountNotFound,
        InvalidBalanceType,
        InvalidBlockSignature,
        SubmitInstructionsFailed,
        SendSignedTransactionError,
        Other,
    }
}

decl_module! {
    pub struct Module<T: Trait> for enum Call where origin: T::Origin {
        fn deposit_event() = default;

        /// Clean the state on initialisation of a block
        fn on_initialize(_now: T::BlockNumber) -> Weight {
            <Self as Store>::OcRequests::kill();
            0
        }

        #[weight = 0]
        pub fn outgoing_transfer(origin, sender: T::AccountId, receiver: iroha::AccountId, asset_kind: AssetKind, amount: u128, nonce: u8) -> DispatchResult {
            debug::debug!("called outgoing_transfer");
            let author = ensure_signed(origin)?;

            if Self::is_authority(&author) {
                <treasury::Module<T>>::burn(sender.clone(), asset_kind, amount);
                debug::info!("Finalized outgoing transfer request {} {:?} from {:?} to {}", amount, asset_kind, sender, receiver);
                Self::deposit_event(RawEvent::OutgoingTransfer(sender, receiver, asset_kind, amount));
            }

            Ok(())
        }

        #[weight = 0]
        pub fn request_transfer(origin, receiver: iroha::AccountId, asset_kind: AssetKind, amount: u128, nonce: u8) -> DispatchResult {
            debug::debug!("called request_transfer");
            let _sender = ensure_signed(origin)?;

            // TODO: transfer from `sender`
            let mut from = <Self as Store>::Accounts::get(&receiver);
            if from == T::AccountId::default() {
                debug::error!("Account was not found for: {:?}", receiver);
                return Err(<Error<T>>::AccountNotFound.into());
            }
            <treasury::Module<T>>::lock(from.clone(), asset_kind, amount);

            <Self as Store>::OcRequests::mutate(|v| v.push(OffchainRequest::OutgoingTransfer(from, receiver, asset_kind, amount, nonce)));
            Ok(())
        }

        #[weight = 0]
        pub fn incoming_transfer(origin, sender: iroha::AccountId, receiver: T::AccountId, asset_kind: treasury::AssetKind, amount: u128) -> DispatchResult {
            debug::debug!("called force_transfer");
            let author = ensure_signed(origin)?;
            if Self::is_authority(&author) {
                debug::info!("Incoming transfer from {} to {:?} with {:?} {:?}", sender, receiver, amount, asset_kind);
                if <Accounts<T>>::get(&sender) == T::AccountId::default() {
                    <Accounts<T>>::insert(sender.clone(), receiver.clone());
                }
                <treasury::Module<T>>::mint(receiver.clone(), asset_kind, amount);
                Self::deposit_event(RawEvent::IncomingTransfer(sender, receiver, asset_kind, amount));
            }
            Ok(())
        }

        #[weight = 0]
        pub fn add_authority(origin, who: T::AccountId) -> DispatchResult {
            let _ = ensure_root(origin)?;
            if !Self::is_authority(&who) {
                <Authorities<T>>::mutate(|l| l.push(who));
            }
            Ok(())
        }

        fn offchain_worker(block_number: T::BlockNumber) {
            debug::info!("Entering off-chain workers");
            for e in <Self as Store>::OcRequests::get() {
                match e {
                    OffchainRequest::OutgoingTransfer(from, to, asset_kind, amount, nonce) => {
                        if let Err(_e) = Self::handle_outgoing_transfer(from, to, asset_kind, amount, nonce) {
                            // TODO: unlock currency
                        }
                    }
                }
            }

            match Self::fetch_iroha() {
                Ok(_) => (),
                Err(e) => { debug::error!("Fetching Iroha error: {:?}", e); }
            }
        }
    }
}

impl<T: Trait> Module<T> {
    fn handle_block(block: ValidBlock) -> Result<(), Error<T>> {
        debug::debug!("Handling Iroha block at height {}", block.header.height);
        for tx in block.transactions {
            let author_id = tx.payload.account_id;
            let bridge_account_id = iroha::AccountId::new("bridge", "polkadot");
            for isi in tx.payload.instructions {
                match isi {
                    iroha::Instruction::Account(AccountInstruction::TransferAsset(
                        from,
                        to,
                        mut asset,
                    )) => {
                        debug::info!(
                            "Outgoing {} transfer from {}",
                            asset.id.definition_id.name,
                            from
                        );
                        if to == bridge_account_id {
                            let asset_kind = AssetKind::try_from(&asset.id.definition_id)
                                .map_err(|_| <Error<T>>::Other)?;
                            use sp_core::crypto::AccountId32;
                            let quantity = asset.quantity;
                            let amount = quantity as u128;

                            let signer = Signer::<T, T::AuthorityId>::any_account();
                            if !signer.can_sign() {
                                debug::error!("No local account available");
                                return Err(<Error<T>>::Other);
                            }

                            let recipient_account = {
                                let mut recipient_account = <Self as Store>::Accounts::get(&from);
                                if recipient_account == T::AccountId::default() {
                                    let account_query = GetAccount::build_request(from.clone());
                                    let query_result = Self::send_query(account_query)?;
                                    debug::trace!("query result: {:?}", query_result);
                                    let queried_acc = match query_result {
                                        iroha::QueryResult::GetAccount(res) => res.account,
                                        _ => return Err(<Error<T>>::Other),
                                    };
                                    let account_pk =
                                        queried_acc.signatories.first().ok_or(<Error<T>>::Other)?;
                                    let account_id =
                                        utils::substrate_account_id_from_iroha_pk::<T>(account_pk);
                                    <Accounts<T>>::insert(from.clone(), account_id.clone());
                                    recipient_account = account_id;
                                }
                                recipient_account
                            };

                            let result = signer.send_signed_transaction(|acc| {
                                debug::debug!("signer {:?}", acc.id);
                                Call::incoming_transfer(
                                    from.clone(),
                                    recipient_account.clone(),
                                    asset_kind,
                                    amount,
                                )
                            });

                            match result {
                                Some((acc, Ok(_))) => {
                                    let bridge_def_id =
                                        BridgeDefinitionId::new(&bridge_account_id.domain_name);
                                    let tx = ExternalTransaction {
                                        hash: "".into(),
                                        payload: vec![],
                                    };
                                    let instructions = vec![bridge::isi::handle_outgoing_transfer(
                                        &bridge_def_id,
                                        &asset_kind.definition_id(),
                                        quantity,
                                        0,
                                        &tx,
                                    )];
                                    let resp = Self::send_instructions(instructions);
                                    if resp.is_err() {
                                        debug::error!(
                                            "error while processing handle_outgoing_transfer ISI"
                                        );
                                        return Err(<Error<T>>::SubmitInstructionsFailed);
                                    } else {
                                        debug::error!("ok processing handle_outgoing_transfer ISI");
                                    }
                                }
                                Some((acc, Err(e))) => {
                                    debug::error!(
                                        "[{:?}] Failed in signed_submit_number: {:?}",
                                        acc.id,
                                        e
                                    );
                                    return Err(<Error<T>>::SendSignedTransactionError);
                                }
                                _ => {
                                    debug::error!("Failed in signed_submit_number");
                                    return Err(<Error<T>>::SendSignedTransactionError);
                                }
                            };
                        }
                    }
                    _ => (),
                }
            }
        }
        Ok(())
    }

    fn fetch_blocks(from_height: u64) -> Result<Vec<ValidBlock>, Error<T>> {
        let null_pk = iroha_crypto::PublicKey::try_from(vec![0u8; 32]).unwrap();
        let mut get_blocks =
            BlockMessage::GetBlocksFromHeight(from_height, PeerId::new("", &null_pk));
        let msg = Self::http_request::<_, BlockMessage>(BLOCK_ENDPOINT, &get_blocks)?;
        let blocks = match msg {
            BlockMessage::ShareBlocks(blocks, _) => blocks,
            _ => {
                debug::error!("Received wrong BlockMessage variant");
                return Err(<Error<T>>::Other);
            }
        };
        for block in blocks.clone() {
            for (pk, sig) in block
                .signatures
                .values()
                .iter()
                .cloned()
                .map(utils::iroha_sig_to_substrate_sig::<T>)
            {
                let block_hash = T::Hashing::hash(&block.header.encode());
                if !T::AuthorityId::verify(block_hash.as_ref(), pk, sig) {
                    debug::error!("Invalid signature of block: {:?}", block_hash);
                    return Err(<Error<T>>::InvalidBlockSignature);
                }
            }
        }
        debug::debug!("Blocks are verified");
        Ok(blocks)
    }

    fn fetch_iroha() -> Result<(), Error<T>> {
        let s_last_fetched_height =
            StorageValueRef::persistent(b"iroha-bridge-ocw::last-fetched-height");
        let s_lock = StorageValueRef::persistent(b"iroha-bridge-ocw::block-fetch-lock");

        let res: Result<Result<bool, bool>, Error<T>> =
            s_lock.mutate(|s: Option<Option<bool>>| match s {
                None | Some(Some(false)) => Ok(true),
                _ => Err(<Error<T>>::AlreadyFetched),
            });

        if let Ok(Ok(true)) = res {
            let latest_height = s_last_fetched_height.get::<u64>().flatten().unwrap_or(0);
            match Self::fetch_blocks(latest_height) {
                Ok(blocks) if !blocks.is_empty() => {
                    // update last block hash and release the lock
                    s_last_fetched_height.set(&blocks.last().unwrap().header.height);
                    s_lock.set(&false);

                    for block in blocks {
                        Self::handle_block(block);
                    }
                    debug::debug!("fetched blocks");
                }
                Ok(_empty) => {
                    // no new blocks received, release the lock
                    s_lock.set(&false);
                }
                Err(err) => {
                    // release the lock
                    s_lock.set(&false);
                    return Err(err);
                }
            }
        }
        Ok(())
    }

    fn send_instructions(instructions: Vec<iroha::Instruction>) -> Result<(), Error<T>> {
        let signer = Signer::<T, T::AuthorityIdEd>::all_accounts();
        if !signer.can_sign() {
            debug::error!("No local account available");
            return Err(<Error<T>>::Other);
        }
        let mut requested_tx = RequestedTransaction::new(
            instructions,
            account::Id::new("root", "global"),
            10000,
            sp_io::offchain::timestamp().unix_millis(),
        );
        let payload_encoded = requested_tx.payload.encode();
        let sigs = signer.sign_message(&payload_encoded);
        for (acc, sig) in sigs {
            debug::trace!("send_instructions acc [{}]: {:?}", acc.index, acc.public);
            if acc.index == 0 {
                let sig = utils::substrate_sig_to_iroha_sig::<T>((acc.public, sig));
                requested_tx.signatures.push(sig);
            }
        }

        let resp = Self::http_request::<_, ()>(INSTRUCTION_ENDPOINT, &requested_tx)?;
        Ok(resp)
    }

    fn send_query(query: iroha::QueryRequest) -> Result<iroha::QueryResult, Error<T>> {
        let signer = Signer::<T, T::AuthorityId>::all_accounts();
        if !signer.can_sign() {
            debug::error!("No local account available");
            return Err(<Error<T>>::Other);
        }
        let query_result = Self::http_request(QUERY_ENDPOINT, &query)?;
        Ok(query_result)
    }

    fn handle_outgoing_transfer(
        from_account_id: T::AccountId,
        to_account_id: iroha::AccountId,
        asset_kind: AssetKind,
        amount: u128,
        nonce: u8,
    ) -> Result<(), Error<T>> {
        debug::info!("Received transfer request");

        let signer = Signer::<T, T::AuthorityId>::all_accounts();
        if !signer.can_sign() {
            debug::error!("No local account available");
            return Err(<Error<T>>::Other);
        }

        let asset_definition_id = asset_kind.definition_id();
        let bridge_def_id = BridgeDefinitionId::new("polkadot");
        let bridge_account_id = iroha::AccountId::new("bridge", &bridge_def_id.name);
        let quantity = u32::try_from(amount).map_err(|_| <Error<T>>::InvalidBalanceType)?;

        let instructions = vec![bridge::isi::handle_incoming_transfer(
            &bridge_def_id,
            &asset_definition_id,
            quantity,
            0,
            to_account_id.clone(),
            &ExternalTransaction {
                hash: "".into(),
                payload: vec![],
            },
        )];
        let resp = Self::send_instructions(instructions);
        if resp.is_err() {
            debug::error!("error while sending instructions");
            return Err(<Error<T>>::SubmitInstructionsFailed);
        }
        let results = signer.send_signed_transaction(|_acc| {
            Call::outgoing_transfer(
                from_account_id.clone(),
                to_account_id.clone(),
                asset_kind,
                amount,
                nonce,
            )
        });

        for (acc, res) in &results {
            match res {
                Ok(()) => {
                    debug::native::trace!("off-chain respond: acc: {:?}| nonce: {}", acc.id, nonce);
                }
                Err(e) => {
                    debug::error!("[{:?}] Failed in respond: {:?}", acc.id, e);
                    return Err(<Error<T>>::SendSignedTransactionError);
                }
            };
        }
        Ok(())
    }

    fn http_request<B: Encode, R: Decode>(url: &str, body: &B) -> Result<R, Error<T>> {
        debug::trace!("Sending request to: {}", url);
        let request = rt_offchain::http::Request::post(url, vec![body.encode()]);
        let timeout = sp_io::offchain::timestamp().add(rt_offchain::Duration::from_millis(10000));
        let pending = request.deadline(timeout).send().map_err(|e| {
            debug::error!("Failed to send a request {:?}", e);
            <Error<T>>::HttpFetchingError
        })?;
        let response = pending
            .try_wait(timeout)
            .map_err(|e| {
                debug::error!("Failed to get a response: {:?}", e);
                <Error<T>>::HttpFetchingError
            })?
            .map_err(|e| {
                debug::error!("Failed to get a response: {:?}", e);
                <Error<T>>::HttpFetchingError
            })?;
        if response.code != 200 {
            debug::error!("Unexpected http request status code: {}", response.code);
            return Err(<Error<T>>::HttpFetchingError);
        }
        let resp = response.body().collect::<Vec<u8>>();
        R::decode(&mut resp.as_slice()).map_err(|e| {
            debug::error!("Failed to decode {}: {:?}", core::any::type_name::<R>(), e);
            <Error<T>>::Other
        })
    }

    fn is_authority(who: &T::AccountId) -> bool {
        Self::authorities().into_iter().find(|i| i == who).is_some()
    }
}
