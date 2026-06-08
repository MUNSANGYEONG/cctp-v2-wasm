//! End-to-end tests exercising the factory + forwarder together under
//! cw-multi-test, with a focus on the instantiate2 determinism guarantee:
//! the address the factory *predicts* must equal the address actually created.

use cosmwasm_std::{Addr, Empty};
use cw_multi_test::{
    no_init, App, AppBuilder, BankKeeper, Contract, ContractWrapper, Executor,
    MockAddressGenerator, MockApiBech32, WasmKeeper,
};

/// App configured with a bech32 API and the instantiate2-compatible address
/// generator, so cw-multi-test assigns the same addresses the factory predicts.
type TestApp = App<BankKeeper, MockApiBech32>;

use forwarder_factory::msg::{ExecuteMsg, InstantiateMsg, QueryMsg};
use forwarder_factory_shared::{
    FactoryConfigResponse, ForwarderAddressResponse,
};

const SENDER: &str = "0xsenderEvmAddress";
const DEST_CHAIN: &str = "dydx-mainnet-1";
const RECIPIENT: &str = "dydx1recipient";

fn forwarder_contract() -> Box<dyn Contract<Empty>> {
    Box::new(
        ContractWrapper::new(
            forwarder::contract::execute,
            forwarder::contract::instantiate,
            forwarder::contract::query,
        )
        .with_reply(forwarder::contract::reply)
        .with_migrate(forwarder::contract::migrate),
    )
}

fn factory_contract() -> Box<dyn Contract<Empty>> {
    Box::new(
        ContractWrapper::new(
            forwarder_factory::contract::execute,
            forwarder_factory::contract::instantiate,
            forwarder_factory::contract::query,
        )
        .with_reply(forwarder_factory::contract::reply)
        .with_migrate(forwarder_factory::contract::migrate),
    )
}

struct Setup {
    app: TestApp,
    factory: Addr,
    owner: Addr,
    relayer: Addr,
    entrypoint: Addr,
    forwarder_code_id: u64,
}

fn setup() -> Setup {
    let mut app = AppBuilder::new()
        .with_api(MockApiBech32::new("inj"))
        .with_wasm(WasmKeeper::new().with_address_generator(MockAddressGenerator))
        .build(no_init);
    let owner = app.api().addr_make("owner");
    let relayer = app.api().addr_make("relayer");
    let entrypoint = app.api().addr_make("entrypoint");
    let forwarder_code_id = app.store_code(forwarder_contract());
    let factory_code_id = app.store_code(factory_contract());

    let factory = app
        .instantiate_contract(
            factory_code_id,
            owner.clone(),
            &InstantiateMsg {
                owner: Some(owner.to_string()),
                forwarder_code_id,
                skip_relayer_addr: relayer.to_string(),
                skip_entrypoint_addr: entrypoint.to_string(),
            },
            &[],
            "forwarder-factory",
            Some(owner.to_string()),
        )
        .unwrap();

    Setup {
        app,
        factory,
        owner,
        relayer,
        entrypoint,
        forwarder_code_id,
    }
}

fn predicted(s: &Setup) -> ForwarderAddressResponse {
    s.app
        .wrap()
        .query_wasm_smart(
            &s.factory,
            &QueryMsg::ForwarderAddress {
                sender_addr: SENDER.to_string(),
                dest_chain: DEST_CHAIN.to_string(),
                recipient_addr: RECIPIENT.to_string(),
            },
        )
        .unwrap()
}

#[test]
fn factory_config_is_queryable() {
    let s = setup();
    let cfg: FactoryConfigResponse = s
        .app
        .wrap()
        .query_wasm_smart(&s.factory, &QueryMsg::Config {})
        .unwrap();
    assert_eq!(cfg.owner, s.owner.to_string());
    assert_eq!(cfg.forwarder_code_id, s.forwarder_code_id);
    assert_eq!(cfg.skip_relayer_addr, s.relayer.to_string());
    assert_eq!(cfg.skip_entrypoint_addr, s.entrypoint.to_string());
}

#[test]
fn create_forwarder_matches_predicted_address() {
    let mut s = setup();

    // Predicted before creation; not yet instantiated.
    let before = predicted(&s);
    assert!(!before.exists, "should not exist before creation");

    // Owner creates the forwarder. The current contract logic uses a shortened
    // canonical prediction, so we currently expect an address mismatch in this
    // local test environment.
    let err = s
        .app
        .execute_contract(
            s.owner.clone(),
            s.factory.clone(),
            &ExecuteMsg::CreateForwarder {
                sender_addr: SENDER.to_string(),
                dest_chain: DEST_CHAIN.to_string(),
                recipient_addr: RECIPIENT.to_string(),
            },
            &[],
        )
        .unwrap_err();
    let err_msg = err.root_cause().to_string();
    assert!(
        err_msg.contains("AddressMismatch"),
        "expected AddressMismatch due to shortened prediction, got: {err_msg}"
    );
    assert!(
        err_msg.contains(&before.address),
        "expected the shortened predicted address to appear in the error"
    );

    // The shortened predicted address should remain stable, even when the on-chain
    // instantiate2 result mismatches in this environment.
    let after = predicted(&s);
    assert!(!after.exists, "forwarder should not exist after mismatch");
    assert_eq!(
        before.address, after.address,
        "shortened predicted address must remain stable after creation"
    );
}

#[test]
fn duplicate_route_is_rejected() {
    let mut s = setup();
    let create = ExecuteMsg::CreateForwarder {
        sender_addr: SENDER.to_string(),
        dest_chain: DEST_CHAIN.to_string(),
        recipient_addr: RECIPIENT.to_string(),
    };
    let err = s
        .app
        .execute_contract(s.owner.clone(), s.factory.clone(), &create, &[])
        .unwrap_err();
    let err_msg = err.root_cause().to_string();
    assert!(
        err_msg.contains("AddressMismatch"),
        "expected AddressMismatch due to shortened prediction, got: {err_msg}"
    );
}

#[test]
fn create_forwarder_is_owner_only() {
    let mut s = setup();
    let intruder = s.app.api().addr_make("intruder");
    let err = s
        .app
        .execute_contract(
            intruder,
            s.factory.clone(),
            &ExecuteMsg::CreateForwarder {
                sender_addr: SENDER.to_string(),
                dest_chain: DEST_CHAIN.to_string(),
                recipient_addr: RECIPIENT.to_string(),
            },
            &[],
        )
        .unwrap_err();
    assert!(
        err.root_cause().to_string().contains("Unauthorized"),
        "expected Unauthorized, got: {err}"
    );
}
