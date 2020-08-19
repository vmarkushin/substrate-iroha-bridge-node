cargo build --package substrate-iroha-bridge-node
cargo build --package iroha
cargo build --package bridge-tester

read -p "Run tester? (Y/n) " -n 1 -r
echo
if [[ $REPLY =~ ^[Yy]$ ]]
then
  ./target/debug/iroha > /dev/null & RUST_LOG=pallet_iroha_bridge=debug ./target/debug/substrate-iroha-bridge-node --tmp --dev --ws-external & sleep 10 && ./target/debug/bridge-tester
fi

bash
