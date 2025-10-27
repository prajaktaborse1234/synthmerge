# synthmerge

**AI-powered conflict resolution for Git with your own workflow**

`synthmerge` is a minimalist, no-GUI command-line tool that leverages AI to resolve conflicts from `git cherry-pick` operations. It leverages the core principles and research of the [Patchpal project](https://gitlab.com/patchpal-ai), but focusing on a pure AI inference layer that works with your existing Git workflow. Reviews of the AI generated code still happen in your favorite editor.

---

## 🌟 Core Philosophy

1. **Separation of concerns**  
   Pure AI inference layer that doesn't duplicate Git functionality

2. **Git dependency**  
   Relies on Git's `diff3` conflict markers (requires `git config merge.conflictStyle diff3`)

3. **Developer workflow freedom**  
   Works with *any* editor (VS Code, Emacs, Vim, etc.)

4. **Model flexibility**  
   Removes the Patchpal fine-tuning requirement, allowing any instruct model to be used

---

## ✨ Key Features

- **Universal Git operation support**  
  Works seamlessly after all Git operations that create conflicts:
  - `cherry-pick`
  - `merge`
  - `rebase`
  - `revert`

- **Parallel Multi-AI endpoint support**  
  Queries multiple AI models simultaneously to resolve conflicts:
  - Patchpal-backend (fine-tuned specifically for conflict resolution)
  - Self-hosted open weight open source LLMs with OpenAI compatible endpoints
  - Gemini (via OpenAI-compatible API)

- **Results deduplication**  
  Combines identical solutions, showing which models agree on what

- **Review using your workflow**  
  - Resolved conflicts appear in your editor with model attribution
  - AI-generated code requires manual review before commit

- **Fail-safe design**  
  If one model fails to resolve a conflict, Git's original conflict remains alongside solutions from other models for that hunk

- **Configurable**  
  Configure inference servers: reasoning effort, temperature, no_context...

---

## 🛠 How It Works

1. **Git sets up conflicts**  
   ```bash
   git config merge.conflictStyle diff3  # Must be set
   git cherry-pick -x <commit>           # Git detects conflicts
   ```

2. **synthmerge analyzes conflicts**  
   - Reads Git's `diff3` conflict markers
   - Extracts context (3 lines before/after conflict)
   - Generates precise AI prompt

3. **AI resolves conflict**  
   - Sends code + patch to configured endpoint
   - Receives resolved code

4. **Git gets updated**  
   - synthmerge inserts the AI resolution into existing diff3 markers
   - You review in your editor

---

## ⚙️ Configuration

Create `~/.config/synthmerge.yaml` based on `synthmerge.yaml`:

```yaml
endpoints:

  - name: "Patchpal AI"
    type: "patchpal"
    url: "http://patchpal.usersys.redhat.com:9080/v1"

  - name: "llama.cpp vulkan"
    url: "http://localhost:8811/v1/chat/completions"
    type: "openai"
    model: "your favorite open weight open source coder model"
    temperature: 0.7

  - name: "llama.cpp vulkan no_context"
    url: "http://localhost:8811/v1/chat/completions"
    type: "openai"
    model: "your favorite open weight open source coder model"
    no_context: true

  - name: "Gemini 2.5 pro"
    url: "https://generativelanguage.googleapis.com/v1beta/openai/chat/completions"
    type: "openai"
    model: "gemini-2.5-pro"
    api_key_file: "~/.gemini-api-key"
    reasoning_effort: "low"
```

---

## 🚀 Usage

```bash
# Ensure Git is configured for diff3
git config merge.conflictStyle diff3

# Attempt cherry-pick (will leave conflicts)
git cherry-pick -x <commit>

# Resolve conflicts with AI
synthmerge

# Review in your editor
git diff --name-only --diff-filter=U
```

---

## 🌐 Supported AI Endpoints

| Endpoint Type | Example Configuration | Notes |
|---------------|------------------------|-------|
| **Patchpal-backend** | `type: "patchpal"` | Fine-tuned for patch resolution |
| **OpenAI protocol** | `type: "openai"` | Self-hosted LLMs (e.g., `llama.cpp`) and Gemini |

> ✅ **Gemini supports a compatible OpenAI endpoint**  
> ✅ **Models work with stock weights** – the prompt engineering simulates Patchpal's fine-tuned behavior.

---

## 🛠 Installation

Build from source:

```bash
git clone https://github.com/aarcange/synthmerge.git
cd synthmerge
cargo build --release
```

---

## License

[![License: GPL-3.0-or-later](https://img.shields.io/badge/License-GPL--3.0--or--later-blue.svg)](https://www.gnu.org/licenses/gpl-3.0.html)
[![License: AGPL-3.0-or-later](https://img.shields.io/badge/License-AGPL--3.0--or--later-blue.svg)](https://www.gnu.org/licenses/agpl-3.0.html)
