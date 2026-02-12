# ⚔️ UKSFTA Platinum DevOps Suite v1.2.0

The **UKSFTA Platinum DevOps Suite** is a high-fidelity unit management and automation toolkit designed for professional Arma 3 development. It provides a "Zero Trust" infrastructure that ensures project stability, security, and performance across the entire unit workspace.

## 🚀 Key Features

### 🌐 Workspace Operations

- **`status`**: Instant git status summary for every unit repository.
- **`sync`**: Automated Workshop dependency synchronization and lockfile management.
- **`update`**: One-click propagation of latest DevOps tools to all unit projects.
- **`self-update`**: Keep your local toolkit synchronized with the master repository.

### 🧠 Unit Intelligence

- **`dashboard`**: A high-level visual overview of all projects, components, and versions.
- **`workshop-info`**: Query live versions, sizes, and timestamps directly from Steam.
- **`modlist-size`**: Calculate the total data footprint of any Arma 3 modlist (HTML or text).
- **`gh-runs`**: Real-time monitoring of all unit CI/CD pipelines.

### 🔍 Assurance & Quality (The Guard)

- **`audit`**: Master command running the full suite of health and security checks.
- **`audit-signatures`**: Automated verification of PBO signing and unit key matching.
- **`audit-security`**: Proactive scanning for leaked tokens, webhooks, or private keys.
- **`audit-performance`**: Texture and model optimization analysis to prevent stuttering.
- **`audit-mission`**: Verify mission PBOs against the local workspace and external dependencies.

### 🏗️ Production & Utilities

- **`release`**: Professional packaging with GPG-signed tags and standardized ZIPs.
- **`mission-setup`**: Standardize mission folders with unit-standard headers and frameworks.
- **`generate-report`**: Create consolidated Markdown health reports for unit leadership.
- **`fix-syntax`**: Automated code formatting and standardization across all repos.
- **`notify`**: Dispatch professional development update cards to Discord.

## 💻 Developer Experience (DX)

- **Git Hooks**: Local pre-commit guards to block security leaks and syntax errors.
- **VS Code Integration**: One-click task menu for all common dev actions.
- **JSON Support**: Global `--json` flag for integration with unit bots and dashboards.

---

## 🛠 Getting Started

1. **Prerequisites**: Ensure you have `python3`, `git`, `hemtt`, and `steamcmd` installed.
2. **Setup**: Run `./tools/workspace_manager.py check-env` to verify your local environment.
3. **Usage**: Run `./tools/workspace_manager.py help` to see the full command suite.

---

### Maintained by the UKSF Taskforce Alpha Development Team
