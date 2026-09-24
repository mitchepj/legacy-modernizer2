use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use legacy_modernizer::llm::{HttpLlmClient, LlmClient};
use legacy_modernizer::state::{ApprovalRecord, LedgerEntry, QualificationRecord, RunLedger};
use legacy_modernizer::{architecture, codegen, config::Config, demo, discovery, document, risk_triage, state, verify};
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(
    name = "legacy-modernizer",
    version,
    about = "Six-agent legacy code modernization orchestrator, built for air-gapped deployment against a self-hosted, OpenAI-compatible LLM endpoint."
)]
struct Cli {
    /// Path to legacy-modernizer.toml.
    #[arg(short, long, default_value = "legacy-modernizer.toml", global = true)]
    config: PathBuf,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Scaffold a starting legacy-modernizer.toml in the current directory.
    Init {
        #[arg(long, default_value = "unnamed-engagement")]
        name: String,
    },
    /// Phase -1: measure the configured model's real usable context
    /// ceiling and record it, before running Discovery on real source.
    Qualify,
    /// Stage 1: Discovery & Inventory. Reads raw source under
    /// `source_dir` and produces 01-discovery.json.
    Discover {
        source_dir: PathBuf,
    },
    /// Stage 2: Risk & Triage. Consumes 01-discovery.json only.
    Triage,
    /// Stage 3: Architecture & Decoupling. Produces 03-architecture.md
    /// and then runs the interactive Phase 2 approval gate.
    Architect,
    /// Stage 4: Code Generation. Refuses to run without a valid,
    /// hash-matching architecture approval on record.
    Generate {
        /// Path to the raw legacy module source to translate in this batch.
        module_source: PathBuf,
    },
    /// Stage 5: Verification. Reviews generated code against the legacy
    /// module and produces a PASS/FAIL/FAIL-WITH-CONDITIONS verdict.
    Verify {
        module_source: PathBuf,
        #[arg(long)]
        generated_file: PathBuf,
    },
    /// Stage 6: Documentation & Audit. Runs last; produces ADRs, a
    /// traceability matrix, and (if configured) a compliance control map.
    Document,
    /// Print what's been run so far for this engagement.
    Status,
    /// Offline, fully scripted six-stage run against a tiny embedded
    /// sample — no LLM endpoint required. Proves the pipeline wiring
    /// works before you point it at a real model.
    Demo {
        #[arg(long, default_value = "demo-output")]
        output_dir: PathBuf,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    // `init` and `demo` don't need a config file to exist yet.
    match &cli.command {
        Command::Init { name } => return cmd_init(&cli.config, name),
        Command::Demo { output_dir } => return cmd_demo(output_dir),
        _ => {}
    }

    let cfg = Config::load(&cli.config)
        .with_context(|| format!("run `legacy-modernizer init` first, or pass --config (tried: {})", cli.config.display()))?;
    let output_dir = PathBuf::from(&cfg.engagement.output_dir);
    std::fs::create_dir_all(&output_dir)?;

    match cli.command {
        Command::Init { .. } | Command::Demo { .. } => unreachable!(),
        Command::Qualify => cmd_qualify(&cfg, &output_dir),
        Command::Discover { source_dir } => cmd_discover(&cfg, &source_dir, &output_dir),
        Command::Triage => cmd_triage(&cfg, &output_dir),
        Command::Architect => cmd_architect(&cfg, &output_dir),
        Command::Generate { module_source } => cmd_generate(&cfg, &module_source, &output_dir),
        Command::Verify { module_source, generated_file } => cmd_verify(&cfg, &module_source, &generated_file, &output_dir),
        Command::Document => cmd_document(&cfg, &output_dir),
        Command::Status => cmd_status(&output_dir),
    }
}

fn make_client(cfg: &Config) -> Result<HttpLlmClient> {
    HttpLlmClient::new(cfg.llm.base_url.clone(), cfg.llm.api_key.clone(), cfg.llm.model.clone(), cfg.llm.temperature)
}

fn cmd_init(config_path: &Path, name: &str) -> Result<()> {
    if config_path.exists() {
        bail!("{} already exists - refusing to overwrite", config_path.display());
    }
    let cfg = Config::scaffold(name);
    let toml_str = toml::to_string_pretty(&cfg)?;
    std::fs::write(config_path, toml_str)?;
    println!("Wrote {}. Edit [llm] to point at your self-hosted OpenAI-compatible endpoint, then run `qualify`.", config_path.display());
    Ok(())
}

fn cmd_demo(output_dir: &Path) -> Result<()> {
    println!("Running the full six-stage pipeline offline against an embedded sample (no LLM endpoint required)...\n");
    let summary = demo::run_full_demo(output_dir)?;
    println!("Discovery:      {}", summary.discovery_report.display());
    println!("Risk & Triage:  {}", summary.risk_triage_report.display());
    println!("Architecture:   {}", summary.architecture_report.display());
    println!("Code Generation:{}", summary.codegen_report.display());
    for f in &summary.generated_files {
        println!("  generated: {}", f.display());
    }
    println!("Verification:   {} (verdict: {:?})", summary.verification_report.display(), summary.verdict);
    println!("Documentation:  {}", summary.documentation_report.display());
    println!("\nDemo complete. All output is under {}.", summary.output_dir.display());
    Ok(())
}

fn cmd_qualify(cfg: &Config, output_dir: &Path) -> Result<()> {
    let client = make_client(cfg)?;
    println!("Probing {} ({})...", cfg.llm.base_url, cfg.llm.model);
    let probe = client
        .chat("You are a diagnostic probe.", "Reply with exactly: OK")
        .context("qualification probe call failed - check base_url/model/network reachability")?;
    println!("Endpoint responded: {}", probe.trim());

    let record = QualificationRecord {
        model: cfg.llm.model.clone(),
        base_url: cfg.llm.base_url.clone(),
        measured_context_ceiling_words: cfg.llm.context_ceiling_words,
        qualified_at: state::now_rfc3339(),
        notes: "Context ceiling is the configured starting hypothesis, not yet empirically re-measured. Increase legacy-modernizer.toml's context_ceiling_words only after confirming the model handles that many words per call in your own testing.".to_string(),
    };
    record.save(output_dir)?;
    println!("Phase -1 qualification recorded. Discovery may now proceed.");
    Ok(())
}

fn require_qualified(output_dir: &Path) -> Result<()> {
    if QualificationRecord::load(output_dir)?.is_none() {
        bail!("no Phase -1 qualification record found - run `legacy-modernizer qualify` first");
    }
    Ok(())
}

fn cmd_discover(cfg: &Config, source_dir: &Path, output_dir: &Path) -> Result<()> {
    require_qualified(output_dir)?;
    let client = make_client(cfg)?;
    let t0 = state::now_rfc3339();
    let report = discovery::run(&client, source_dir, output_dir, &cfg.engagement.extra_excluded_dirs, cfg.llm.context_ceiling_words)?;
    RunLedger::append(output_dir, LedgerEntry {
        stage: "discovery".to_string(),
        started_at: t0,
        finished_at: state::now_rfc3339(),
        status: "ok".to_string(),
        detail: format!("wrote {}", report.display()),
    })?;
    println!("Discovery complete: {}", report.display());
    Ok(())
}

fn load_json(path: &Path) -> Result<serde_json::Value> {
    let raw = std::fs::read_to_string(path).with_context(|| format!("could not read {}", path.display()))?;
    serde_json::from_str(&raw).with_context(|| format!("could not parse {} as JSON", path.display()))
}

fn cmd_triage(cfg: &Config, output_dir: &Path) -> Result<()> {
    let discovery_path = output_dir.join(discovery::REPORT_FILENAME);
    if !discovery_path.exists() {
        bail!("{} not found - run `discover` first", discovery_path.display());
    }
    let discovery_json = load_json(&discovery_path)?;
    let client = make_client(cfg)?;
    let report = risk_triage::run(&client, &discovery_json, output_dir)?;
    println!("Risk & Triage complete: {}", report.display());
    Ok(())
}

fn cmd_architect(cfg: &Config, output_dir: &Path) -> Result<()> {
    let discovery_path = output_dir.join(discovery::REPORT_FILENAME);
    let risk_path = output_dir.join(risk_triage::REPORT_FILENAME);
    if !discovery_path.exists() || !risk_path.exists() {
        bail!("run `discover` and `triage` first");
    }
    let discovery_json = load_json(&discovery_path)?;
    let risk_json = load_json(&risk_path)?;
    let client = make_client(cfg)?;
    let report = architecture::run(&client, &discovery_json, &risk_json, output_dir)?;
    println!("Architecture proposal written: {}", report.display());

    architecture::approve_interactive(&report, output_dir, cfg.approvers.phase2_default_approver.as_deref())?;
    Ok(())
}

fn cmd_generate(cfg: &Config, module_source: &Path, output_dir: &Path) -> Result<()> {
    let architecture_path = output_dir.join(architecture::REPORT_FILENAME);
    if !ApprovalRecord::is_valid_for(output_dir, &architecture_path)? {
        bail!(
            "no valid Phase 2 approval on record for {} - run `architect` and approve it first (if the document changed since approval, it must be re-approved)",
            architecture_path.display()
        );
    }
    let architecture_md = std::fs::read_to_string(&architecture_path)?;
    let module_code = std::fs::read_to_string(module_source)
        .with_context(|| format!("could not read {}", module_source.display()))?;

    let client = make_client(cfg)?;
    let result = codegen::run(&client, &architecture_md, &module_code, output_dir)?;
    println!("Code Generation complete: {}", result.report_path.display());
    for f in &result.files_written {
        println!("  wrote: {}", f.display());
    }
    Ok(())
}

fn cmd_verify(cfg: &Config, module_source: &Path, generated_file: &Path, output_dir: &Path) -> Result<()> {
    let architecture_path = output_dir.join(architecture::REPORT_FILENAME);
    let architecture_md = std::fs::read_to_string(&architecture_path)
        .with_context(|| format!("could not read {} - run `architect` first", architecture_path.display()))?;
    let legacy = std::fs::read_to_string(module_source)?;
    let migrated = std::fs::read_to_string(generated_file)?;

    let client = make_client(cfg)?;
    let result = verify::run(&client, &legacy, &migrated, &architecture_md, output_dir)?;
    println!("Verification complete: {} (verdict: {:?})", result.report_path.display(), result.verdict);
    Ok(())
}

fn cmd_document(cfg: &Config, output_dir: &Path) -> Result<()> {
    let discovery_json = std::fs::read_to_string(output_dir.join(discovery::REPORT_FILENAME))?;
    let risk_json = std::fs::read_to_string(output_dir.join(risk_triage::REPORT_FILENAME))?;
    let architecture_md = std::fs::read_to_string(output_dir.join(architecture::REPORT_FILENAME))?;
    let codegen_md = std::fs::read_to_string(output_dir.join(codegen::REPORT_FILENAME))?;
    let verification_md = std::fs::read_to_string(output_dir.join(verify::REPORT_FILENAME))?;

    let history = document::PipelineHistory {
        discovery_json: &discovery_json,
        risk_triage_json: &risk_json,
        architecture_md: &architecture_md,
        codegen_notes_md: &codegen_md,
        verification_md: &verification_md,
        compliance_regime: cfg.engagement.compliance_regime.as_deref(),
    };

    let client = make_client(cfg)?;
    let report = document::run(&client, &history, output_dir)?;
    println!("Documentation & Audit complete: {}", report.display());
    Ok(())
}

fn cmd_status(output_dir: &Path) -> Result<()> {
    let ledger = RunLedger::load(output_dir)?;
    if ledger.entries.is_empty() {
        println!("No stages recorded yet under {}.", output_dir.display());
    }
    for e in &ledger.entries {
        println!("[{}] {} - {} ({})", e.finished_at, e.stage, e.status, e.detail);
    }
    for (label, path) in [
        ("Discovery", output_dir.join(discovery::REPORT_FILENAME)),
        ("Risk & Triage", output_dir.join(risk_triage::REPORT_FILENAME)),
        ("Architecture", output_dir.join(architecture::REPORT_FILENAME)),
        ("Code Generation", output_dir.join(codegen::REPORT_FILENAME)),
        ("Verification", output_dir.join(verify::REPORT_FILENAME)),
        ("Documentation", output_dir.join(document::REPORT_FILENAME)),
    ] {
        println!("{label}: {}", if path.exists() { "done" } else { "not run" });
    }
    match QualificationRecord::load(output_dir)? {
        Some(q) => println!("Phase -1 qualification: {} @ {} (recorded {})", q.model, q.base_url, q.qualified_at),
        None => println!("Phase -1 qualification: not recorded"),
    }
    match ApprovalRecord::load(output_dir)? {
        Some(a) => println!("Phase 2 approval: {} at {} ({})", a.approver, a.approved_at, a.notes),
        None => println!("Phase 2 approval: none on record"),
    }
    Ok(())
}
