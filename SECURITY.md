# Security Policy

The Stellar Tags / SorobanKit team takes the security of our payment platform, smart contracts, backend services, and frontend applications very seriously. We appreciate the responsible disclosure of vulnerabilities by researchers and the community.

## Supported Versions

We provide security updates and patches for the following versions:

| Version | Supported          | Notes                                     |
| ------- | ------------------ | ----------------------------------------- |
| `1.x` (`main`) | :white_check_mark: | Active development & production releases  |
| `< 1.0` | :x:                | Deprecated / EOL                          |

---

## Reporting a Vulnerability

If you discover a potential security vulnerability within this repository, **do not open a public GitHub issue, pull request, or discussion**. Public disclosure exposes users and contracts to unnecessary risk before a mitigation is available.

Please report vulnerabilities using one of the following channels:

### 1. GitHub Private Vulnerability Reporting (Preferred)
Submit a confidential advisory directly through GitHub:
- Navigate to the [GitHub Security Advisory page](https://github.com/SorobanKit/sorobanKit/security/advisories/new).
- Provide a summary, detailed description, and reproduction steps.
- Collaborators and security maintainers will privately triage and collaborate with you on the patch.

### 2. Security Contact Email
If you prefer email or cannot use GitHub advisories, please contact:
- **Email:** `security@stellartags.com` (or `security@sorobankit.org`)
- **Subject:** `[Vulnerability Report] Component - Brief Summary`

### What to Include in Your Report
To help us triage and resolve the issue quickly, please provide:
1. **Component affected:** Frontend (`payment-dashboard`), Backend API (`stellar-payment-platform`), or Soroban Contract (`payment_router`).
2. **Severity assessment:** Explanation of potential impact (e.g., unauthorized funds transfer, privilege escalation, data leakage, denial of service).
3. **Step-by-step reproduction instructions:** Minimal Proof of Concept (PoC) code, curl requests, or transaction payloads.
4. **Proposed mitigation:** Suggested code changes or workarounds (if known).

---

## Response Time Commitments

We are committed to handling all vulnerability reports promptly and transparently:

* **Initial Acknowledgement:** Within **48 hours** of receiving your report.
* **Triage & Validation:** Within **5 business days**, confirming whether the issue is reproducible and determining severity.
* **Remediation & Fix Deployment:** Within **14 calendar days** for Critical and High severity vulnerabilities. Moderate and Low severity issues will be addressed in the next scheduled release cycle.
* **Coordinated Public Disclosure:** Once the fix has been tested and deployed to production/mainnet, we will coordinate a release advisory and credit the reporter.

---

## Safe Harbor & Responsible Disclosure

If you follow these responsible disclosure guidelines:
* We will not pursue legal action against you or contact law enforcement regarding your security research activities.
* We ask that you give us reasonable time to investigate and mitigate the vulnerability before publicly disclosing any details.
* Do not exploit vulnerabilities to view, modify, or exfiltrate customer data or drain liquidity beyond what is necessary to prove the concept.

---

## Bug Bounties

Stellar Tags does not currently run an automated paid bug bounty program. However, researchers who report valid, high-impact security vulnerabilities in accordance with this policy will be:
- Publicly acknowledged in our release notes and Security Hall of Fame (with your permission).
- Nominated for community recognition and bounty allocations via Stellar Community Fund and ecosystem developer programs where applicable.

Thank you for helping keep Stellar Tags and our community safe!
