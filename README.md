# legacy-modernizer

A standalone, compiled implementation of the six-agent **Legacy
Modernization Orchestrator**: Discovery & Inventory → Risk & Triage →
Architecture & Decoupling → Code Generation → Verification & Test →
Documentation & Audit.

It is built specifically for **air-gapped deployment**: a single static
Linux binary that talks to a **self-hosted, OpenAI-compatible LLM
endpoint** (vLLM, llama.cpp's server, Ollama, TGI, LM Studio, or anything
else that speaks the `POST /v1/chat/completions` shape). No network
access is required at runtime beyond that one endpoint, and none of the
tool's guardrails, prompts, or safety constraints live in a config file
that could be edited away — they are compiled into the binary (see
`src/prompts.rs`).

This tool automates the *orchestration* — sequencing, human-approval
gates, file I/O, report generation — around six LLM calls. **It cannot,
and does not try to, compile "the agents" themselves**: the actual
reasoning at each stage still comes from whatever model you point it at.
What's compiled here is the harness: the safety preamble every call
gets, the immutable risk matrix, the gate that blocks Code Generation
until a human has approved the architecture, and the parsing that turns
model output into real files on disk.

## Try it with zero setup

```sh
cargo build --release
./target/release/legacy-modernizer demo --output-dir demo-output
```

This runs all six stages against a tiny embedded synthetic C snippet,
using a scripted mock model (no network, no LLM server) — it exists to
prove the pipeline wiring itself works before you point it at a real
model. The same logic is exercised by `cargo test` (see
`tests/prototype_demo.rs`), which also asserts the Phase 2
approval-hash integrity check actually blocks a tampered approval.

## The six stages

| # | Stage | Input | Output | Gate |
|---|---|---|---|---|
| -1 | Qualify | configured `[llm]` endpoint | `qualification.json` | none — this *is* the prerequisite gate for Discovery |
| 1 | Discover | raw legacy source directory | `01-discovery.json` | requires Phase -1 qualification |
| 2 | Triage | `01-discovery.json` | `02-risk-triage.json` | none (automatic scoring against the fixed risk matrix) |
| 3 | Architect | discovery + triage output | `03-architecture.md` | **interactive terminal prompt** — a human must type `y` and their name |
| 4 | Generate | approved architecture + legacy module | `04-codegen.md` + generated source files | refuses to run without a valid, hash-matching approval on record |
| 5 | Verify | legacy module + generated code | `05-verification.md` + generated tests | none automated (this stage *is* the gate — it hands you a PASS/FAIL verdict) |
| 6 | Document | full run history | `06-documentation.md` | runs only after Verification |

Every stage's system prompt is prefixed with the same non-negotiable
safety preamble (`src/prompts.rs::SAFETY_PREAMBLE`): never modify a
legacy file, never invent facts about the codebase, refuse to bypass a
NO-GO or a human gate, prefer flagging for manual engineering over
guessing.

## CLI usage against a real engagement

```sh
# 1. Scaffold a config file, then edit [llm] to point at your endpoint.
legacy-modernizer init --name my-engagement
$EDITOR legacy-modernizer.toml

# 2. Phase -1: qualify the model (probes the endpoint, records the run).
legacy-modernizer qualify

# 3. Discovery: point it at the legacy source tree (never modified).
legacy-modernizer discover /path/to/legacy/source

# 4. Risk & Triage: automatic, scored against the fixed matrix.
legacy-modernizer triage

# 5. Architecture: produces a proposal, then blocks on your approval.
legacy-modernizer architect
#   -> "Approve this architecture proposal ... ? [y/N]: y"
#   -> "Approver name: Jane Doe"

# 6. Code Generation: one module/batch at a time, only after approval.
legacy-modernizer generate /path/to/legacy/source/some_module.c

# 7. Verification: hard gate, PASS/FAIL/FAIL-WITH-CONDITIONS.
legacy-modernizer verify /path/to/legacy/source/some_module.c \
    --generated-file modernization-output/generated/some_module.rs

# 8. Documentation & Audit: ADRs, traceability matrix, compliance mapping.
legacy-modernizer document

# Any time: see what's been run and what's pending.
legacy-modernizer status
```

Everything is written under `[engagement].output_dir` (default
`modernization-output/`) — never into the legacy source tree.

## Configuration (`legacy-modernizer.toml`)

```toml
[llm]
base_url = "http://127.0.0.1:8000/v1"   # OpenAI-compatible; no trailing /chat/completions
api_key = ""                             # optional; most self-hosted servers don't need one
model = "your-served-model-name"
temperature = 0.2
context_ceiling_words = 2000             # starting hypothesis -- re-measure in Phase -1

[engagement]
name = "my-engagement"
output_dir = "modernization-output"
compliance_regime = "NIST 800-53"        # optional; omit for no compliance mapping
extra_excluded_dirs = []                 # on top of built-in defaults (.git, node_modules, target, ...)

[approvers]
phase2_default_approver = "Jane Doe"     # optional suggested name at the approval prompt
```

The safety preamble and the risk matrix are **not** in this file, on
purpose — they're compiled constants (`src/prompts.rs`). Changing them
means forking the tool's guardrails, which should go through a code
change and review, not a config edit.

## Why Rust, and how the binary stays air-gap-friendly

- `reqwest` is built with `default-features = false, features =
  ["blocking", "json", "rustls-tls"]` specifically to avoid a dependency
  on the target machine's system OpenSSL — TLS is statically linked in.
- The release profile (`opt-level = "z"`, `lto = true`, `strip = true`,
  `codegen-units = 1`, `panic = "abort"`) favors a small, self-contained
  binary over compile speed.
- The CI release workflow (see below) additionally cross-compiles a
  **musl** target (`x86_64-unknown-linux-musl`), which statically links
  libc too — that build has zero dynamic library dependencies and will
  run on any x86_64 Linux machine regardless of its glibc version. This
  is the artifact to carry into an air-gapped server whose exact distro
  you don't control.
- `Cargo.lock` is committed deliberately (see `.gitignore`): this is a
  binary, and a build whose provenance matters for audit purposes wants
  pinned dependency versions, not floating ones.

## Building and testing locally

```sh
cargo build --release          # compile
cargo clippy --all-targets     # lint
cargo test                     # unit tests + the offline prototype-demo integration test
```

## Security & safety notes

- This tool **never writes to, modifies, or overwrites any file under
  the legacy source directory you point it at.** All output is new,
  parallel material under `[engagement].output_dir`.
- The Phase 2 (Architecture → Code Generation) gate is enforced twice:
  once as an interactive terminal prompt, and once structurally — the
  `generate` command independently checks a SHA-256 hash of the approved
  architecture document against the document currently on disk, and
  refuses to run if they don't match (i.e. if the document changed after
  sign-off, approval is invalidated and must happen again).
- No red-line risk finding (hardware register access, inline assembly,
  direct interrupt handling, hard real-time deadlines) can be
  overridden by anything in the tool's input — the safety preamble
  explicitly instructs every model call to refuse such a request and
  name the rule that blocks it.
- At runtime this tool makes exactly one kind of network call: `POST
  {base_url}/chat/completions` to the endpoint you configure. It does
  not phone home, check for updates, or fetch anything else.

## License

Apache-2.0 — see `LICENSE`.

---

# Packaging this repository and publishing it on GitHub

Everything below is the step-by-step process for turning this directory
into a repository on your own GitHub account (or organization), publishing
a compiled release, and getting that release into an air-gapped
environment.

## 1. Create the repository on GitHub

1. Sign in to GitHub and click **New repository** (or run `gh repo create` if you have the GitHub CLI installed and authenticated).
2. Name it (e.g. `legacy-modernizer`), choose **Public** or **Private**
   depending on who you want to share it with, and do **not** initialize
   it with a README, license, or `.gitignore` — this directory already
   has all three.

Using the web UI, you'll get a page showing a remote URL like:
`https://github.com/<your-username>/legacy-modernizer.git`

## 2. Push this code to it

From inside this directory:

```sh
git init
git add -A
git commit -m "Initial commit: legacy-modernizer six-agent orchestrator"
git branch -M main
git remote add origin https://github.com/<your-username>/legacy-modernizer.git
git push -u origin main
```

(If you use SSH remotes instead of HTTPS, use
`git@github.com:<your-username>/legacy-modernizer.git` instead.)

Before your first push, edit `Cargo.toml`'s `repository = "..."` field
to point at your actual repository URL, and commit that change too.

## 3. Tag and publish a release (builds the binary via CI)

The included `.github/workflows/release.yml` runs entirely on GitHub's
own cloud runners — it builds the binary and uploads it to a GitHub
Release. **This is the only place `cargo build` ever runs against the
open internet; the air-gapped environment never runs Cargo or reaches
crates.io.**

```sh
git tag v0.1.0
git push origin v0.1.0
```

Pushing a tag matching `v*.*.*` triggers the workflow automatically. Watch
it run under your repository's **Actions** tab. When it finishes, it
publishes a **Release** (under the **Releases** tab) with these files
attached:

- `legacy-modernizer-linux-x86_64-musl` — fully static; use this one for
  an air-gapped Linux server of unknown/unconfirmed glibc version.
- `legacy-modernizer-linux-x86_64-musl.sha256` — its checksum.
- `legacy-modernizer-linux-x86_64-gnu` — standard glibc build.
- `legacy-modernizer-linux-x86_64-gnu.sha256` — its checksum.

## 4. Download the release binary (on a machine with internet access)

```sh
curl -LO https://github.com/<your-username>/legacy-modernizer/releases/download/v0.1.0/legacy-modernizer-linux-x86_64-musl
curl -LO https://github.com/<your-username>/legacy-modernizer/releases/download/v0.1.0/legacy-modernizer-linux-x86_64-musl.sha256
sha256sum -c legacy-modernizer-linux-x86_64-musl.sha256
chmod +x legacy-modernizer-linux-x86_64-musl
```

`sha256sum -c` must print `OK` before you proceed — if it doesn't, the
file was corrupted or tampered with in transit; re-download rather than
carrying it across the air gap.

## 5. Carry the binary across the air gap

Use whatever your organization's approved transfer process is for
moving a file across the boundary (write-once media, a mediated file
transfer/diode system, an approved USB workflow with malware scanning,
etc.) — this tool has no opinion on that process, only on verifying
integrity before and after:

```sh
# On the air-gapped side, after transfer, re-verify before trusting it:
sha256sum legacy-modernizer-linux-x86_64-musl
# Compare this output by hand against the .sha256 file's contents
# (carried across separately, or read off the GitHub Release page on
# the internet-connected side and transcribed/typed in) before running it.
```

Only one file needs to cross the boundary to run the tool: the binary
itself. `legacy-modernizer.toml` is created fresh on the air-gapped side
(step 6) since it names that environment's own LLM endpoint.

## 6. Configure and run it in the air-gapped environment

```sh
mkdir -p ~/legacy-modernizer && cd ~/legacy-modernizer
mv /path/to/legacy-modernizer-linux-x86_64-musl ./legacy-modernizer
chmod +x ./legacy-modernizer

./legacy-modernizer init --name my-air-gapped-engagement
```

Edit the generated `legacy-modernizer.toml`: set `[llm].base_url` to your
self-hosted model server's address (e.g. `http://127.0.0.1:8000/v1` for a
vLLM instance running on the same box, or an internal hostname/IP for one
running elsewhere on the air-gapped network), and `[llm].model` to
whatever name that server serves it under.

Then run Phase -1 qualification before anything else, per the master
design's mandatory gate for air-gapped engagements:

```sh
./legacy-modernizer qualify
```

If that succeeds, proceed through Discovery → Triage → Architecture →
(your approval) → Generation → Verification → Documentation exactly as
in the **CLI usage** section above, pointing `discover` at your real
legacy source tree.

## Updating the tool later

Repeat steps 1–3 are one-time; for a new version, bump `version` in
`Cargo.toml`, commit, tag a new version (`git tag v0.2.0 && git push
origin v0.2.0`), and repeat steps 4–5 to bring the new binary across the
air gap. `legacy-modernizer.toml` and everything under
`[engagement].output_dir` from a prior run are untouched by a binary
upgrade — they're just files next to it.

## Sharing this with others

Once pushed, anyone you give access to the repository (or, if public,
anyone) can:

- clone it and run `cargo build --release && cargo test` themselves, or
- download a pre-built binary directly from your **Releases** page
  without needing Rust installed at all.

If you want to hand it to a colleague who will also run it air-gapped,
send them this README (or just the repository URL, if their side has
one connected machine to read it from) — steps 4–6 above are exactly
what they'll need to do on their own environment.
