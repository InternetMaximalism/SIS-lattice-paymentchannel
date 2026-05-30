# SIS Lattice Payment Channel

This repository contains a prototype multi-party payment channel built around SIS-based private balance commitments.

The current workspace includes:

- `channel-types`: shared channel state and hashing types
- `proof-adapter`: a thin adapter around the upstream SIS amount proof system
- `channel-state`: off-chain channel update construction and verification logic

The upstream proof dependency is pinned to:

- `InternetMaximalism/SIS-lattice-private-balance`
- revision `8eedf2d01cf8b0295bebcb6a330ef79eac6bd95f`

## Benchmarks

The following numbers are local `--release` measurements taken against the pinned upstream revision above. They are machine-dependent, but they are representative of the current proving cost in this workspace.

### `research-candidate`

- `prepare_system`: `684.425916ms`
- `prove`: `422.399333ms`
- `verify`: `13.146292ms`
- `proof size`: `429850 bytes`

### `fast-benchmark`

- `prepare_system`: `49.999291ms`
- `prove`: `26.437708ms`
- `verify`: `2.479667ms`
- `proof size`: `96483 bytes`

These measurements correspond to the upstream benchmark flow in release mode, including both the default research profile and the `fast-benchmark` feature profile.
