# Contributing

Thanks for your interest in contributing! This guide covers local setup, running the test suites, and pull request conventions.

## Prerequisites

| Tool | Version | Notes |
| --- | --- | --- |
| Node.js | >= 18.x (LTS) | Required for the frontend and tooling |
| npm | >= 9.x | Ships with Node 18+ |
| Rust | >= 1.74 (stable) | Required for the Soroban contracts |
| Docker | >= 24.x | Required for the local stack |
| Docker Compose | >= 2.x | Bundled with modern Docker Desktop |
| Stellar CLI | >= 21.x | Required to build/deploy contracts |

Install the Stellar CLI with:

```bash
cargo install --locked stellar-cli --features opt
```

## Environment Setup

1. Clone the repository and install frontend dependencies:

   ```bash
   git clone <repo-url>
   cd <repo>
   npm install
   ```

2. Copy the example environment file and fill in the values:

   ```bash
   cp .env.example .env
   ```

   Key variables:

   | Variable | Description |
   | --- | --- |
   | `DATABASE_URL` | Postgres connection string used by the backend |
   | `STELLAR_NETWORK` | `testnet` or `mainnet` |
   | `HORIZON_URL` | Horizon endpoint for the target network |
   | `SOROBAN_RPC_URL` | Soroban RPC endpoint |
   | `CONTRACT_ID` | Deployed contract ID |
   | `VITE_API_URL` | Backend URL consumed by the frontend |

3. Start the local stack with Docker Compose. Profiles let you run only what you need:

   | Profile | Services | Use case |
   | --- | --- | --- |
   | `default` | backend, postgres | Backend development |
   | `frontend` | frontend, backend, postgres | Full-stack development |
   | `contracts` | stellar quickstart, postgres | Contract development |
   | `all` | every service | End-to-end testing |

   ```bash
   docker compose --profile frontend up --build
   ```

## Running Tests

### Frontend

```bash
npm test
```

### Backend

```bash
cargo test --manifest-path backend/Cargo.toml
```

### Contracts

```bash
cargo test --manifest-path contracts/Cargo.toml
```

## Pull Request Conventions

- Branch from `main` and keep changes focused on a single issue.
- Use conventional commit messages, e.g. `fix: handle Horizon 404 responses`.
- Ensure all relevant test suites pass before opening a PR.
- Reference the issue being addressed in the PR description (`Closes #<issue>`).
- Keep PRs small and reviewable; avoid unrelated refactors.

## Security

If you discover a potential security vulnerability, please do NOT create a public issue or discussion. Follow our responsible disclosure process documented in [SECURITY.md](SECURITY.md).

