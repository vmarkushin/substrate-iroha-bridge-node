use crate::{mock::*, KEY_TYPE, KEY_TYPE_2};
use frame_support::{assert_ok, traits::OnInitialize};
use sp_core::{crypto::AccountId32, sr25519, Pair, Public, ed25519};
use sp_runtime::{
    traits::{Dispatchable, IdentifyAccount, Verify},
    MultiSignature as Signature,
};

use async_std::task;

use iroha::{config::Configuration, prelude, bridge};
use iroha_client::client::account::by_id;
use iroha_client::{client::Client, config::Configuration as ClientConfiguration};
use iroha_client_no_std::prelude as no_std_prelude;
use iroha_client_no_std::crypto as iroha_crypto_no_std;
use parity_scale_codec::alloc::sync::Arc;
use parity_scale_codec::Decode;
use parking_lot::RwLock;
use sp_core::{
    offchain::{OffchainExt, TransactionPoolExt},
    testing::KeyStore,
    traits::KeystoreExt,
};
use sp_io::TestExternalities;
use std::thread;
use tempfile::TempDir;

use sp_core::offchain::Timestamp;

use treasury::AssetKind;
use frame_support::sp_std::convert::TryFrom;
use iroha::prelude::{AccountId as IrohaAccountId, Asset, Account, Domain, AssetDefinition, BridgeDefinitionId, BridgeKind, BridgeDefinition, BridgeId, AssetDefinitionId, Register, Mint, AssetId, Add, Instruction};
use iroha::permission::{Permission, permission_asset_definition_id};
use iroha_crypto::multihash::{Multihash, DigestFunction};
use sp_std::collections::btree_map::BTreeMap;
use iroha::bridge::asset::ExternalAsset;
use iroha_crypto::PrivateKey;
use iroha::peer::PeerId;

pub type SubstrateAccountId = <<Signature as Verify>::Signer as IdentifyAccount>::AccountId;

pub struct ExtBuilder;

impl ExtBuilder {
    pub fn build() -> (
        TestExternalities,
        Arc<RwLock<PoolState>>,
        Arc<RwLock<OffchainState>>,
    ) {
        use sp_runtime::BuildStorage;

        let (offchain, offchain_state) = TestOffchainExt::new();
        let (pool, pool_state) = TestTransactionPoolExt::new();
        let keystore = KeyStore::new();
        {
            let mut guard = keystore.write();
            guard
                .ed25519_generate_new(KEY_TYPE_2, Some("//Alice"))
                .unwrap();
            guard
                .sr25519_generate_new(KEY_TYPE, Some("//Alice"))
                .unwrap();
            guard.sr25519_generate_new(KEY_TYPE, Some("//Bob")).unwrap();
        }
        let _root_account = get_account_id_from_seed::<sr25519::Public>("Alice");
        let endowed_accounts = vec![
            get_account_id_from_seed::<sr25519::Public>("Alice"),
            get_account_id_from_seed::<sr25519::Public>("Bob"),
        ];

        let storage = GenesisConfig {
            system: Some(frame_system::GenesisConfig::default()),
            pallet_balances_Instance1: Some(XORConfig {
                balances: endowed_accounts
                    .iter()
                    .cloned()
                    .filter(|x| {
                        x != &AccountId32::from([
                            52u8, 45, 84, 67, 137, 84, 47, 252, 35, 59, 237, 44, 144, 70, 71, 206,
                            243, 67, 8, 115, 247, 189, 204, 26, 181, 226, 232, 81, 123, 12, 81,
                            120,
                        ])
                    })
                    .map(|k| (k, 0))
                    .collect(),
            }),
            pallet_balances_Instance2: Some(DOTConfig {
                balances: endowed_accounts
                    .iter()
                    .cloned()
                    .map(|k| (k, 1 << 8))
                    .collect(),
            }),
            pallet_balances_Instance3: Some(KSMConfig {
                balances: endowed_accounts
                    .iter()
                    .cloned()
                    .map(|k| (k, 1 << 8))
                    .collect(),
            }),
            pallet_balances: Some(BalancesConfig {
                balances: endowed_accounts
                    .iter()
                    .cloned()
                    .map(|k| (k, 1 << 60))
                    .collect(),
            }),
            // pallet_sudo: Some(SudoConfig { key: root_key }),
            iroha_bridge: Some(IrohaBridgeConfig {
                authorities: endowed_accounts.clone(),
                iroha_peers: vec![iroha_crypto_no_std::PublicKey::try_from(&iroha_crypto_no_std::Multihash {
                    payload: vec![52u8, 45, 84, 67, 137, 84, 47, 252, 35, 59, 237, 44, 144, 70, 71, 206, 243, 67, 8, 115, 247, 189, 204, 26, 181, 226, 232, 81, 123, 12, 81, 120],
                    digest_function: iroha_crypto_no_std::DigestFunction::Ed25519Pub,
                }).unwrap()],
            }),
        }
        .build_storage()
        .unwrap();

        let mut t = TestExternalities::from(storage);
        t.register_extension(OffchainExt::new(offchain));
        t.register_extension(TransactionPoolExt::new(pool));
        t.register_extension(KeystoreExt(keystore));
        t.execute_with(|| System::set_block_number(1));
        (t, pool_state, offchain_state)
    }
}

pub fn get_from_seed<TPublic: Public>(seed: &str) -> <TPublic::Pair as Pair>::Public {
    TPublic::Pair::from_string(&format!("//{}", seed), None)
        .expect("static values are valid; qed")
        .public()
}

type AccountPublic = <Signature as Verify>::Signer;

/// Helper function to generate an account ID from seed
pub fn get_account_id_from_seed<TPublic: Public>(seed: &str) -> SubstrateAccountId
where
    AccountPublic: From<<TPublic::Pair as Pair>::Public>,
{
    AccountPublic::from(get_from_seed::<TPublic>(seed)).into_account()
}

fn create_and_start_iroha() {
    let temp_dir = TempDir::new().expect("Failed to create TempDir.");
    let mut configuration =
        Configuration::from_path("config.json").expect("Failed to load configuration.");
    configuration
        .kura_configuration
        .kura_block_store_path(temp_dir.path());
    let iroha = prelude::Iroha::new(configuration);
    task::block_on(iroha.start()).expect("Failed to start Iroha.");
    //Prevents temp_dir from clean up untill the end of the tests.
    #[allow(clippy::empty_loop)]
    loop {}
}

/// A utility function for our tests. It simulates what the system module does for us (almost
/// analogous to `finalize_block`).
///
/// This function increments the block number and simulates what we have written in
/// `decl_module` as `fn offchain_worker(_now: T::BlockNumber)`: run the offchain logic if the
/// current node is an authority.
///
/// Also, since the offchain code might submit some transactions, it queries the transaction
/// queue and dispatches any submitted transaction. This is also needed because it is a
/// non-runtime logic (transaction queue) which needs to mocked inside a runtime test.
fn seal_block(n: u64, state: Arc<RwLock<PoolState>>, _oc_state: Arc<RwLock<OffchainState>>) {
    assert_eq!(System::block_number(), n);
    System::set_block_number(n + 1);
    IrohaBridge::offchain();

    let transactions = &mut state.write().transactions;
    while let Some(t) = transactions.pop() {
        let e: TestExtrinsic = Decode::decode(&mut &*t).unwrap();
        let (who, _) = e.signature.unwrap();
        let call = e.call;
        // in reality you would do `e.apply`, but this is a test. we assume we don't care
        // about validation etc.
        let _ = call.dispatch(Some(who).into()).unwrap();
    }
    IrohaBridge::on_initialize(System::block_number());
}

fn offchain_worker_loop(oc_state: Arc<RwLock<OffchainState>>) {
    tokio::runtime::Builder::new()
        .basic_scheduler()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async move {
            loop {
                {
                    let mut fulfilled_requests = vec![];
                    let mut reqs = vec![];
                    // I <3 tokio
                    {
                        let mut guard = oc_state.write();
                        guard.timestamp = Timestamp::from_unix_millis(
                            std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap()
                                .as_millis() as u64,
                        );
                    }
                    {
                        let guard = oc_state.read();
                        let pending_requests = &guard.requests;
                        for (id, pending_request) in pending_requests {
                            if pending_request.sent && pending_request.response.is_none() {
                                reqs.push((id.0, pending_request.clone()));
                            }
                        }
                    }
                    for (id, pending_request) in reqs {
                        let bytes = reqwest::Client::new()
                            .post(&pending_request.uri)
                            .body(pending_request.body.clone())
                            .send()
                            .await
                            .unwrap()
                            .bytes()
                            .await
                            .unwrap()
                            .to_vec();

                        fulfilled_requests.push((id, pending_request, bytes));
                    }
                    {
                        let mut guard = oc_state.write();
                        for (id, request, bytes) in fulfilled_requests {
                            guard.fulfill_pending_request(id, request, bytes, vec![]);
                        }
                    }
                }
                thread::sleep(std::time::Duration::from_millis(100));
            }
        });
}

fn check_response_assets(response: &prelude::QueryResult, expected_xor_amount: u32) {
    if let prelude::QueryResult::GetAccount(get_account_result) = response {
        let account = &get_account_result.account;
        let assets = &account.assets;
        let xor_amount = assets
            .iter()
            .find(|(_, asset)| asset.id.definition_id.name == "XOR")
            .map(|(_, asset)| asset.quantity)
            .unwrap_or(0);
        assert_eq!(xor_amount, expected_xor_amount);
        println!(
            "{} account balance on Iroha is: {} XOR",
            account.id, expected_xor_amount
        );
    } else {
        panic!("insufficient XOR amount");
    }
}

async fn init_bridge(iroha_client: &Client) {
    let domain_name = "global";
    let bridge_admin_account_id = IrohaAccountId::new("bridge_admin", domain_name);
    let permission_asset_definition_id = permission_asset_definition_id();
    let bridge_admin_asset_id = AssetId {
        definition_id: permission_asset_definition_id,
        account_id: bridge_admin_account_id.clone(),
    };
    let bridge_permissions_asset =
        Asset::with_permission(bridge_admin_asset_id.clone(), Permission::Anything);
    let bpk = iroha_crypto::PublicKey::try_from(&Multihash {
        digest_function: DigestFunction::Ed25519Pub,
        payload: vec![52, 80, 113, 218, 85, 229, 220, 206, 250, 170, 68, 3, 57, 65, 94, 249, 242, 102,
                      51, 56, 163, 143, 125, 160, 223, 33, 190, 90, 180, 224, 85, 239]
    }).unwrap();
    // let bsk = PrivateKey::try_from(&Multihash {
    //     digest_function: DigestFunction::Ed25519Pub,
    //     payload: vec![250, 199, 149, 157, 191, 231, 47, 5, 46, 90, 12, 60, 141, 101, 48, 242, 2, 176, 47,
    //                   216, 249, 245, 202, 53, 128, 236, 141, 235, 119, 151, 71, 158, 52, 80, 113, 218,
    //                   85, 229, 220, 206, 250, 170, 68, 3, 57, 65, 94, 249, 242, 102, 51, 56, 163, 143,
    //                   125, 160, 223, 33, 190, 90, 180, 224, 85, 239, ]
    // }).unwrap();
    // println!("pk {:?}", bpk.inner);
    // println!("sk {:?}", bsk.inner);
    let mut bridge_admin_account = Account::with_signatory(
        &bridge_admin_account_id.name,
        &bridge_admin_account_id.domain_name,
        bpk.clone(),
    );
    bridge_admin_account
        .assets
        .insert(bridge_admin_asset_id, bridge_permissions_asset);
    let register_bridge_admin_account = Register::new(bridge_admin_account, domain_name.to_owned()).into();
    let peer_id = PeerId::new("", &bpk);
    let add_bridge_domain = Add::new(Domain::new("bridge".into()), peer_id.clone()).into();
    /*
    accounts.insert(bridge_admin_account_id.clone(), bridge_admin_account);
    let domain = Domain {
        name: domain_name.into(),
        accounts,
        asset_definitions,
    };     */
    // let mut domains = BTreeMap::new();
    {
        // let bridge_domain_name = "bridge".to_string();
        /*
        let mut bridge_asset_definitions = BTreeMap::new();
        */
        let asset_definition_ids = [
            bridge::bridges_asset_definition_id(),
            bridge::bridge_asset_definition_id(),
            bridge::bridge_external_assets_asset_definition_id(),
            bridge::bridge_incoming_external_transactions_asset_definition_id(),
            bridge::bridge_outgoing_external_transactions_asset_definition_id(),
        ];
        // let mut instructions: Vec<Instruction> = asset_definition_ids.into_iter()
        //     .map(|x| Register::new(x, domain_name.to_owned()).into());
        /*

for asset_definition_id in &asset_definition_ids {
    bridge_asset_definitions.insert(
        asset_definition_id.clone(),
        AssetDefinition::new(asset_definition_id.clone()),
    );
}
let bridge_domain = Domain {
    name: bridge_domain_name.clone(),
    accounts: BTreeMap::new(),
    asset_definitions: bridge_asset_definitions,
};
// domains.insert(domain_name, domain);
domains.insert(bridge_domain_name, bridge_domain);
*/
        ///
        let bridge_domain_name = "polkadot".to_string();
        let bridge_def_id = BridgeDefinitionId {
            name: bridge_domain_name.clone(),
        };
        let bridge_def = BridgeDefinition {
            id: bridge_def_id.clone(),
            kind: BridgeKind::IClaim,
            owner_account_id: bridge_admin_account_id.clone(),
        };
        let ext_asset = ExternalAsset {
            bridge_id: BridgeId::new(&bridge_def_id.name),
            name: "DOT".to_string(),
            id: "DOT".to_string(),
            decimals: 10,
        };
        let register_bridge = bridge::isi::register_bridge(peer_id, &bridge_def);
        let register_client = bridge::isi::add_client(&bridge_def_id, iroha_client.key_pair.public_key.clone());
        let dot_asset_def = AssetDefinition::new(AssetDefinitionId {
            name: "DOT".to_string(),
            domain_name: bridge_domain_name.clone(),
        });
        let register_dot_asset =
            Register::new(dot_asset_def, bridge_domain_name.clone()).into();
        let xor_asset_def = AssetDefinition::new(AssetDefinitionId {
            name: "XOR".to_string(),
            domain_name: "global".into(),
        });
        let register_xor_asset = Register::new(xor_asset_def.clone(), domain_name.to_owned()).into();
        let register_ext_asset = bridge::isi::register_external_asset(&ext_asset);
        let account_id = IrohaAccountId::new("root", domain_name);
        let mint_xor = Mint::new(
            100u32,
            AssetId::new(xor_asset_def.id.clone(), account_id.clone()),
        )
            .into();
        // let kp = KeyPair {
        //     public_key: pk,
        //     private_key: sk,
        // };

        let instructions = vec![
            register_bridge_admin_account,
            add_bridge_domain,
            Register {
                object: AssetDefinition::new(bridge::bridges_asset_definition_id()),
                destination_id: "bridge".to_owned(),
            }.into(),
            Register {
                object: AssetDefinition::new(bridge::bridge_asset_definition_id()),
                destination_id: "bridge".to_owned(),
            }.into(),
            Register {
                object: AssetDefinition::new(bridge::bridge_external_assets_asset_definition_id()),
                destination_id: "bridge".to_owned(),
            }.into(),
            Register {
                object: AssetDefinition::new(bridge::bridge_incoming_external_transactions_asset_definition_id()),
                destination_id: "bridge".to_owned(),
            }.into(),
            Register {
                object: AssetDefinition::new(bridge::bridge_outgoing_external_transactions_asset_definition_id()),
                destination_id: "bridge".to_owned(),
            }.into(),
            register_xor_asset,
            register_bridge,
            register_client,
            register_dot_asset,
            register_ext_asset,
            mint_xor,
        ];
        iroha_client.submit_all(instructions).await.unwrap();
    }
}

#[async_std::test]
async fn should_transfer_asset_between_iroha_and_substrate() {
    thread::spawn(create_and_start_iroha);
    thread::sleep(std::time::Duration::from_secs(30));

    let configuration =
        ClientConfiguration::from_path("config.json").expect("Failed to load configuration.");
    let mut iroha_client = Client::new(&configuration);
    init_bridge(&iroha_client).await;
    thread::sleep(std::time::Duration::from_secs(5));

    let substrate_user_account =
        AccountId32::decode(&mut &configuration.public_key.payload[..]).unwrap();

    let bridge_account_id = prelude::AccountId::new("bridge", "polkadot");
    let get_bridge_account = by_id(bridge_account_id.clone());
    let response = iroha_client
        .request(&get_bridge_account)
        .await
        .expect("Failed to send request.");
    check_response_assets(&response, 0);

    let global_domain_name = "global";
    let user_account_id = prelude::AccountId::new("root".into(), global_domain_name);
    let get_user_account = by_id(user_account_id.clone());
    let response = iroha_client
        .request(&get_user_account)
        .await
        .expect("Failed to send request.");
    check_response_assets(&response, 100);
    let xor_asset_def = prelude::AssetDefinition::new(prelude::AssetDefinitionId {
        name: "XOR".into(),
        domain_name: global_domain_name.into(),
    });
    let iroha_transfer_xor = prelude::Transfer::new(
        user_account_id.clone(),
        prelude::Asset::with_quantity(
            prelude::AssetId::new(xor_asset_def.id.clone(), user_account_id.clone()),
            100,
        ),
        bridge_account_id.clone(),
    )
    .into();
    iroha_client
        .submit(iroha_transfer_xor)
        .await
        .expect("Failed to send request");
    thread::sleep(std::time::Duration::from_secs(3));

    let (mut ext, state, oc_state) = ExtBuilder::build();

    let oc_state_clone = oc_state.clone();

    let no_std_user_account_id = no_std_prelude::AccountId {
        name: user_account_id.name.clone(),
        domain_name: user_account_id.domain_name.clone(),
    };
    thread::spawn(|| offchain_worker_loop(oc_state_clone));
    ext.execute_with(|| {
        let substrate_balance = Treasury::get_balance_from_account(substrate_user_account.clone(), AssetKind::XOR).unwrap();
        assert_eq!(substrate_balance, 0);

        seal_block(1, state.clone(), oc_state.clone());
        seal_block(2, state.clone(), oc_state.clone());

        let substrate_balance = Treasury::get_balance_from_account(substrate_user_account.clone(), AssetKind::XOR).unwrap();
        assert_eq!(substrate_balance, 100);
    });

    let get_bridge_account = by_id(bridge_account_id.clone());
    let response = iroha_client
        .request(&get_bridge_account)
        .await
        .expect("Failed to send request.");
    check_response_assets(&response, 0);

    let get_user_account = by_id(user_account_id.clone());
    let response = iroha_client
        .request(&get_user_account)
        .await
        .expect("Failed to send request.");
    check_response_assets(&response, 0);

    ext.execute_with(|| {
        let amount = 100u128;
        let nonce = 0u8;
        assert_ok!(IrohaBridge::request_transfer(
            Some(substrate_user_account.clone()).into(),
            no_std_user_account_id.clone(),
            AssetKind::XOR,
            amount,
            nonce
        ));

        seal_block(3, state.clone(), oc_state.clone());
        seal_block(4, state.clone(), oc_state.clone());

        let substrate_balance = Treasury::get_balance_from_account(substrate_user_account.clone(), AssetKind::XOR).unwrap();
        assert_eq!(substrate_balance, 0);
    });
    thread::sleep(std::time::Duration::from_secs(10));

    let get_user_account = by_id(user_account_id.clone());
    let response = iroha_client
        .request(&get_user_account)
        .await
        .expect("Failed to send request.");
    check_response_assets(&response, 100);
}
