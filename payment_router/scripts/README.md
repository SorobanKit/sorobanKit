# Payment Router Development Scripts

This directory contains utility scripts used during local development of the Soroban `payment_router` smart contract.

## `extract_tests.py`

A helper script used to split inline `#[cfg(test)] mod test { ... }` blocks from `src/lib.rs` into a standalone `src/test.rs` file.

### Usage

Run from the `payment_router` directory:
```bash
python scripts/extract_tests.py
```

Or from the workspace root:
```bash
python payment_router/scripts/extract_tests.py
```
