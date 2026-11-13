# Test fixtures

- `mock_state.json`, `delegated_anchor.json` — Mock backend fixtures.
- `centralized_wire.json` — Centralized write-signature vectors (CS-06).
  - The sr25519 key is a **throwaway test key**; the private key is not stored.
  - Each vector contains `op`, `body`, `canonical`, `payload` (`op || "\n" || canonical`)
    and `signature_hex`; `tests/centralized_wire_fixtures.rs` re-computes the canonical
    bytes and verifies the signature against `public_key_hex`.
  - Servers implementing the wire protocol can use these vectors for byte-for-byte
    compatibility tests.
