# Read-only mobile M0 spike

Throwaway prototype, not a production pairing protocol. No provider login, core command proxy, execution, approval or terminal input. Do not deploy this loopback relay or host publicly.

## Run

Requires an existing Node 22+ and local `qrencode` executable. No npm installation. The QR utility is invoked directly with stdin; it receives a local invitation, not uploaded data. Its LGPL-2.1-or-later packaging/licensing must be resolved before bundling; it is not bundled here.

1. Use a temporary HOME. Create a local read-model input JSON containing typed `projects`, `sessions`, `attention` pairs `[session_id, count]`, and explicit project-ID `scope`.
2. Run `cargo run --manifest-path app/core/Cargo.toml --example mobile_m0_snapshot --offline < input.json > status.json` using existing toolchain homes and a temporary Cargo target directory. This bounds input at 256 KiB; the core exporter validates records and emits at most 64 KiB.
3. In a second terminal: `node spikes/mobile-m0/relay.mjs` (loopback 8788; choose `M0_RELAY_PORT` if occupied).
4. In another: `M0_SNAPSHOT=/absolute/temp/status.json M0_HOST_PORT=0 node spikes/mobile-m0/host.mjs`. The program prints the loopback desktop URL. `M0_RELAY` selects the relay origin.
5. Open desktop URL, click Show pairing QR; open the local test-client link (equivalent invitation to scanning the QR on a client that can reach the address). Compare codes, confirm on desktop. Projects/runs/needs-you counts render in the read-only client.
6. Click Unpair / revoke. Host destroys pair references and clears its timer, asks relay to remove mailbox, and stops publishing. Client clears stale content within 15 seconds even if an uncooperative relay retains old ciphertext. Prior plaintext screenshots cannot be revoked.

The default URL is loopback: a physical phone cannot reach it. This proves the minimal **web-client** path. Real phone testing needs separately approved HTTPS hosting/client distribution and relay origin policies, not changing the listener to 0.0.0.0. The browser client's keys live in memory and are lost on refresh; no durable device store or native keychain integration is claimed. No change to the Tauri app's CSP/capabilities, guard, sandbox, or existing action APIs.

The desktop pairing view is a local web harness, not a settings panel in the shipped desktop binary. Exporter is library+example code against typed contracts; it is not connected to a live project store. Until a trustworthy project store exists, copying the old app-global state into every project is unsafe.

## Verify

Set `HOME` to a newly created `/tmp/vajra-mobile-*` directory, then `node --test spikes/mobile-m0/spike.test.mjs`. Tests bind only loopback ephemeral ports and use fixture files. Core projection tests: `cargo test --manifest-path app/core/Cargo.toml --lib mobile_m0 --offline`, also with temporary HOME, explicit existing RUSTUP_HOME/CARGO_HOME and a temporary target directory.

The relay sees routing IDs, role capabilities, timing/size, connection addresses and ciphertext. It never receives invitation secrets, endpoint ECDH private keys or plaintext status. It is not blind to metadata. Compromised JS delivery, either endpoint, or local OS defeats endpoint confidentiality. Phone UI contains no input controls; its only write is encrypted pairing hello, not an app action.

Production must use a reviewed authenticated handshake/library, owner-authenticated native pairing and scope selection, device-secure key storage, authenticated leases/reconnect, robust relay abuse limits, key rotation, and a real mobile release. This prototype has no independent cryptographic audit or production security claim.
