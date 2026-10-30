# Strawberry Social: Comprehensive Engineering Specification

## 1. Project Overview & Objectives

**Strawberry Social** is a lightweight, high-performance, local-first AI chat front-end built in Rust. It serves as a streamlined, low-friction alternative to resource-heavy tools like SillyTavern. By trading massive configuration complexity for zero-configuration defaults and a unified binary release, Strawberry Social provides an optimized experience for running roleplay and persona chat locally on CPU using GGUF models via `llama.cpp` or Koboldcpp.

### Core Philosophy

* **Zero Configuration Out-of-the-Box:** Reasonable defaults allow users to launch and chat immediately.
* **Self-Contained Deployment:** Distributed as a single executable (`.exe`/binary) containing all backend logic and embedded frontend assets.
* **Direct Integration:** Designed natively to ingest and render SillyTavern Character Cards (V1, V2, and V3 formats) and embedded PNG metadata produced by companion tools (such as the Python GTK card editor referenced in the ecosystem).

---

## 2. Technical Requirements & Architecture

* **Language:** Rust (Latest stable edition).
* **Build System:** Cargo exclusively. No Node.js, npm, or external build pipelines for the web client.
* **Distribution:** Single executable binary. Static embedding of frontend assets via `rust-embed`.
* **Configuration:** Single-file YAML configuration (`config.yaml`) created automatically on first run.
* **Cross-Platform:** Linux, Windows, and macOS compatible.
* **Runtime Constraints:** CPU-only execution; relies strictly on external local endpoints (`llama.cpp` or Koboldcpp server APIs). No heavy C++ weight bindings compiled into the binary.
* **Data Storage:** Flat-file asset directories (`/characters`, `/sessions`) combined with an embedded SQLite database (`rusqlite`) for fast, robust chat session tracking and long-term memory logs.

---

## 3. System Architecture & Folder Layout

Upon initial execution, the application initializes the following local directory structure relative to the binary path:

```text
strawberry-social/
├── strawberry-social(.exe)    # Single compiled binary
├── config.yaml                # Global application configuration
├── characters/                # Storage for JSON and PNG character cards
│   ├── avatars/               # Extracted or companion card portraits
│   └── lorebooks/             # Associated character book JSONs
└── sessions/                  # SQLite database or JSONL session logs
    └── chats.db

```

---

## 4. Detailed Functional & Component Specifications

### 4.1 Backend Engine (Rust)

#### Core Modules:

1. **`main.rs` & Server (`axum` or `actix-web`):** Spins up a local HTTP server on a dynamic or configured loopback port (default: `127.0.0.1:8080`), automatically opening the user's default browser or serving the SPA interface.
2. **Card Parser (`card_model.rs` counterpart in Rust):**
* Implements robust parsing for V1, V2 (`chara_card_v2`), and V3 (`chara_card_v3`) structures.
* Reads raw PNG binary byte chunks (`tEXt`/`iTXt`) looking for `ccv3` or `chara` keys, decodes base64 JSON, and validates mandatory fields (`name`, `description`, `personality`, `first_mes`, etc.).

3. **LLM Client Layer:**
* Communicates with local endpoints (`[http://127.0.0.1:5001/v1/chat/completions](http://127.0.0.1:5001/v1/chat/completions)` or Koboldcpp equivalents).
* Manages streaming Server-Sent Events (SSE) back to the web client for real-time token rendering.

4. **Session & Memory Store:**
* Tracks active chat threads, message histories, persona bindings, and extensions via SQLite.

### 4.2 Frontend Interface (Vanilla Web Stack)

* **Tech Stack:** Modern vanilla JavaScript (ES modules), HTML5, and CSS3 (utilizing CSS Custom Properties for clean theming). No framework bloat.
* **UI Layout:**
* **Sidebar:** Character Roster selector (grid/list view rendering card avatars and names), active chat session history list, and global settings toggle.
* **Main Chat Workspace:** Message history pane with Markdown rendering (`marked.js` or lightweight parser), streaming response cursor, user input text area with auto-grow, and quick-action toolbars (regenerate, edit message, branch chat).
* **Inspector Drawer (Collapsible):** Displays current character details (`description`, `personality`, `scenario`, active `character_book` entries, and token usage estimates).

---

## 5. Data Schemas & Models

### 5.1 Global Configuration (`config.yaml`)

```yaml
server:
  host: "127.0.0.1"
  port: 8080
  open_browser_on_start: true

llm:
  api_base_url: "http://127.0.0.1:5001/v1" # Compatible with llama.cpp / Koboldcpp
  api_key: "not-needed"
  default_model: "local-model"
  context_window: 4096
  temperature: 0.7
  top_p: 0.9

ui:
  theme: "dark"
  stream_responses: true

```

### 5.2 Character Card V2/V3 Ingestion Spec (Rust Struct Mapping)

The backend must natively ingest cards adhering to the standard schema produced by companion card editors:

* **Core Identity:** `name`, `description`, `personality`, `scenario`, `first_mes`, `mes_example`.
* **Prompt Overrides:** `system_prompt`, `post_history_instructions`, `creator_notes`.
* **Metadata & Extensions:** `alternate_greetings` (array), `tags` (array), `creator`, `character_version`, `extensions` (map), and optional `character_book` (Lorebook entries with keys, content, and insertion parameters).

---

## 6. Implementation Milestones for LLM Agent Construction

When building or prompting an AI agent to scaffold this codebase, direct execution should follow these sequential phases:

1. **Phase 1: Rust Backend Core & Static Server**
* Initialize Cargo project, configure dependencies (`axum`, `tokio`, `serde`, `serde_yaml`, `rusqlite`, `reqwest`, `image`, `rust-embed`).
* Implement file-system bootstrapping (`characters/`, `sessions/`, default `config.yaml`).
* Set up embedded static file serving for the web client.

2. **Phase 2: Character Card Parser Module**
* Port robust JSON and PNG chunk-parsing logic (handling base64-encoded `chara` and `ccv3` chunks) into Rust to extract metadata and validate structures seamlessly.

3. **Phase 3: LLM Proxy & Streaming API**
* Build REST endpoints (`/api/chat`, `/api/characters`, `/api/sessions`) bridging the web UI to local `llama.cpp`/Koboldcpp backends with SSE support.

4. **Phase 4: Vanilla Frontend SPA**
* Construct clean, responsive HTML/CSS/JS layouts inside `/static`.
* Implement roster grid loading, chat session switching, markdown message rendering, and character detail inspection drawers.
