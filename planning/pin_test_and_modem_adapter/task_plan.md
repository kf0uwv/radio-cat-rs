# Task plan: pin_test_and_modem_adapter
1. [x] Read ts570d/src/bin/pin_test.rs and ft991a/src/main.rs in full.
2. [x] Build cat-transport-serial/src/bin/pin_test.rs ([[bin]] "pin-test").
3. [x] Build cat_transport_core::NoModemControlLines with unit tests.
4. [x] Fold both into docs/adr/0006-windows-network-transport.md (§6/§7).
5. [x] cargo fmt/clippy/test; Windows cross-compile checks.
