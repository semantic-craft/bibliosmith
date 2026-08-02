use super::*;

#[test]
fn stderr_tail_surfaces_the_last_lines_of_a_python_traceback() {
    let stderr = "Traceback (most recent call last):\n  File \"x.py\", line 1\nRuntimeError: GOOGLE_API_KEY or GEMINI_API_KEY not set.\n";
    assert_eq!(
        stderr_tail(stderr),
        "Traceback (most recent call last): | File \"x.py\", line 1 | RuntimeError: GOOGLE_API_KEY or GEMINI_API_KEY not set."
    );
}

#[test]
fn stderr_tail_ignores_blank_lines_and_caps_at_three() {
    let stderr = "\n  \nfirst\nsecond\nthird\nfourth\n";
    assert_eq!(stderr_tail(stderr), "second | third | fourth");
}

#[test]
fn stderr_tail_is_empty_for_empty_stderr() {
    assert_eq!(stderr_tail(""), "");
}

#[test]
fn stderr_tail_redacts_an_auth_header_but_not_a_missing_key_message() {
    let stderr =
        "connecting...\nAuthorization: Bearer sk-abc123\nRuntimeError: GEMINI_API_KEY not set.\n";
    let tail = stderr_tail(stderr);
    assert!(!tail.contains("sk-abc123"), "leaked a secret: {tail}");
    assert!(
        tail.contains("GEMINI_API_KEY not set"),
        "over-redacted a message that names no secret: {tail}"
    );
}

#[test]
fn stderr_tail_redacts_a_key_assignment() {
    let stderr = "DASHSCOPE_API_KEY=sk-abc123\n";
    let tail = stderr_tail(stderr);
    assert!(!tail.contains("sk-abc123"), "leaked a secret: {tail}");
}

fn executable_fixture(dir: &Path, name: &str) -> PathBuf {
    fs::create_dir_all(dir).unwrap();
    let path = dir.join(name);
    fs::write(&path, "#!/bin/sh\n").unwrap();
    path
}

#[test]
fn program_search_dirs_keep_inherited_path_ahead_of_the_desktop_fallbacks() {
    let inherited = env::join_paths(["/usr/bin", "/bin"].map(PathBuf::from)).unwrap();
    let dirs = program_search_dirs_from(
        Some(inherited.as_os_str()),
        vec![PathBuf::from("/opt/homebrew/bin")],
    );
    assert_eq!(
        dirs,
        vec![
            PathBuf::from("/usr/bin"),
            PathBuf::from("/bin"),
            PathBuf::from("/opt/homebrew/bin"),
        ]
    );
}

#[test]
fn program_search_dirs_drop_duplicates_and_empty_entries() {
    let inherited = env::join_paths(["/usr/bin", "", "/opt/homebrew/bin"].map(PathBuf::from))
        .unwrap_or_else(|_| OsString::from("/usr/bin::/opt/homebrew/bin"));
    let dirs = program_search_dirs_from(
        Some(inherited.as_os_str()),
        vec![PathBuf::from("/opt/homebrew/bin"), PathBuf::from("/bin")],
    );
    assert_eq!(
        dirs,
        vec![
            PathBuf::from("/usr/bin"),
            PathBuf::from("/opt/homebrew/bin"),
            PathBuf::from("/bin"),
        ]
    );
}

#[test]
fn program_search_dirs_survive_a_desktop_launch_without_path() {
    let dirs = program_search_dirs_from(None, vec![PathBuf::from("/opt/homebrew/bin")]);
    assert_eq!(dirs, vec![PathBuf::from("/opt/homebrew/bin")]);
}

#[test]
fn only_real_ocr_commands_request_keychain_credentials() {
    let output = PathBuf::from("/tmp/book-pipeline-command-scope");
    let fake = RunnerCommand {
        kind: RunnerCommandKind::Fake,
        label: "fake Book Pipeline runner".into(),
        program: PathBuf::from("fake"),
        args: Vec::new(),
        env: Vec::new(),
        cwd: None,
        output_dir: output,
        attempts: 0,
        accepted_exit_codes: vec![0],
    };
    assert!(!command_uses_ocr_credentials(&fake));

    let mut command = fake.clone();
    for label in [
        ZOTERO_CONVERSION_COMMAND_LABEL,
        "MinerU Precision batch",
        "local PDF conversion wrapper",
    ] {
        command.label = label.into();
        assert!(command_uses_ocr_credentials(&command), "{label}");
    }
    command.label = "external Book Pipeline adapter".into();
    assert!(!command_uses_ocr_credentials(&command));
}

#[test]
fn public_state_prefers_a_digest_verified_mineru_source_over_the_old_route_label() {
    let root = temp_root("current-mineru-evidence");
    let project = root.join("project");
    fs::create_dir_all(project.join("source/source.mineru")).unwrap();
    fs::create_dir_all(project.join("metadata")).unwrap();
    let source = project.join("source/source.md");
    fs::write(&source, "# MinerU source\n").unwrap();
    fs::write(
        project.join("source/source.mineru/mineru_manifest.json"),
        "{}\n",
    )
    .unwrap();
    fs::write(
        project.join("metadata/source_manifest.json"),
        serde_json::to_string(&serde_json::json!({
            "source_sha256": sha256_file(&source).unwrap(),
            "extraction_engine": "MinerU Precision v4 VLM",
            "mineru_manifest_path": "source/source.mineru/mineru_manifest.json",
        }))
        .unwrap(),
    )
    .unwrap();
    let mut job = mineru_overlay_fixture_job(&project);

    overlay_current_mineru_source_evidence(&mut job);

    let route = &job.children[0].route[0];
    assert_eq!(route.route_kind, "mineru");
    assert!(route.summary.contains("MinerU Precision v4 VLM"));
    assert!(route.summary.contains("direct_text"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn mineru_source_overlay_rejects_a_stale_source_digest() {
    let root = temp_root("stale-mineru-evidence");
    let project = root.join("project");
    fs::create_dir_all(project.join("source/source.mineru")).unwrap();
    fs::create_dir_all(project.join("metadata")).unwrap();
    fs::write(project.join("source/source.md"), "# Changed source\n").unwrap();
    fs::write(
        project.join("source/source.mineru/mineru_manifest.json"),
        "{}\n",
    )
    .unwrap();
    fs::write(
        project.join("metadata/source_manifest.json"),
        r#"{"source_sha256":"deadbeef","extraction_engine":"MinerU Precision v4 VLM","mineru_manifest_path":"source/source.mineru/mineru_manifest.json"}"#,
    )
    .unwrap();
    let mut job = mineru_overlay_fixture_job(&project);

    overlay_current_mineru_source_evidence(&mut job);

    assert_eq!(job.children[0].route[0].route_kind, "direct_text");
    let _ = fs::remove_dir_all(root);
}

fn mineru_overlay_fixture_job(project: &Path) -> BookPipelineJob {
    serde_json::from_value(serde_json::json!({
        "id": "job-mineru-overlay",
        "mode": "convert_then_translate",
        "source": { "kind": "zotero_attachment", "routeOverrides": {} },
        "route": [],
        "status": "completed",
        "currentStep": "Completed",
        "lastError": null,
        "logSummary": [],
        "artifacts": [],
        "outputDir": null,
        "attempts": 1,
        "children": [{
            "id": "child-mineru-overlay",
            "parentJobId": "job-mineru-overlay",
            "status": "completed",
            "currentStageId": "validate_reading",
            "source": { "kind": "zotero_attachment", "routeOverrides": {} },
            "route": [{
                "id": "book",
                "title": "Book",
                "sourceKind": "zotero_attachment",
                "sourceRef": "zotero://attachment/BOOK",
                "routeKind": "direct_text",
                "canRun": true,
                "blockedReason": null,
                "summary": "Direct embedded text extraction"
            }],
            "localProjectRoot": display_path(project)
        }],
        "createdAt": "2026-07-30T00:00:00Z",
        "updatedAt": "2026-07-30T00:00:00Z"
    }))
    .unwrap()
}

// The launchd default a Finder-launched .app inherits holds no uv and no
// node, so a bare name has to resolve out of the fallback roots instead.
#[test]
fn runner_program_resolves_a_bare_name_a_desktop_path_cannot_reach() {
    let root = temp_root("program-lookup");
    let desktop_path = root.join("usr-bin");
    let homebrew = root.join("homebrew-bin");
    fs::create_dir_all(&desktop_path).unwrap();
    let uv = executable_fixture(&homebrew, "uv");

    let dirs = program_search_dirs_from(
        Some(env::join_paths([&desktop_path]).unwrap().as_os_str()),
        vec![homebrew.clone()],
    );
    assert_eq!(
        resolve_runner_program_in(&dirs, Path::new("uv")),
        uv,
        "a bare uv should resolve out of the fallback roots"
    );

    fs::remove_dir_all(&root).ok();
}

#[test]
fn runner_program_keeps_an_explicit_path_untouched() {
    let root = temp_root("program-explicit");
    let bin = root.join("bin");
    let shadow = executable_fixture(&bin, "python3");
    let explicit = PathBuf::from("/opt/homebrew/bin/python3.11");

    let dirs = vec![bin.clone()];
    assert_eq!(
        resolve_runner_program_in(&dirs, &explicit),
        explicit,
        "an explicit interpreter choice must not be re-resolved"
    );
    assert_ne!(resolve_runner_program_in(&dirs, &explicit), shadow);

    fs::remove_dir_all(&root).ok();
}

// Falling back to the bare name keeps the spawn error naming the tool the
// caller asked for rather than an invented path.
#[test]
fn runner_program_falls_back_to_the_bare_name_when_nothing_resolves() {
    let root = temp_root("program-missing");
    fs::create_dir_all(&root).unwrap();
    assert_eq!(
        resolve_runner_program_in(&[root.clone()], Path::new("uv")),
        PathBuf::from("uv")
    );
    fs::remove_dir_all(&root).ok();
}

#[test]
fn runner_child_path_carries_every_search_dir() {
    let value = runner_path_env_value().expect("search dirs should join into a PATH");
    let carried = env::split_paths(&value).collect::<Vec<_>>();
    assert_eq!(carried, program_search_dirs());
}

fn nvm_fixture(root: &Path, versions: &[&str], default_alias: Option<&str>) -> PathBuf {
    let nvm_root = root.join(".nvm");
    for version in versions {
        fs::create_dir_all(
            nvm_root
                .join("versions")
                .join("node")
                .join(version)
                .join("bin"),
        )
        .unwrap();
    }
    if let Some(alias) = default_alias {
        let alias_dir = nvm_root.join("alias");
        fs::create_dir_all(&alias_dir).unwrap();
        fs::write(alias_dir.join("default"), format!("{alias}\n")).unwrap();
    }
    nvm_root
}

// A machine whose only node came from nvm has it under a versioned directory
// no constant can spell, so `build_reading` used to fail to spawn there.
#[test]
fn nvm_bin_dirs_lead_with_the_default_alias() {
    let root = temp_root("nvm-default-alias");
    let nvm_root = nvm_fixture(&root, &["v18.20.4", "v22.11.0", "v24.17.0"], Some("22"));

    let dirs = nvm_bin_dirs(&nvm_root);

    assert_eq!(
        dirs.first(),
        Some(&nvm_root.join("versions/node/v22.11.0/bin")),
        "the default alias should win over the newest install"
    );
    assert_eq!(dirs.len(), 3, "the other installs stay as fallbacks");
    fs::remove_dir_all(&root).ok();
}

// This machine's own `~/.nvm/alias/default` says `22` while only v24 is
// installed; naming nothing installed must not leave the search without node.
#[test]
fn nvm_bin_dirs_fall_back_to_the_newest_install() {
    let root = temp_root("nvm-stale-alias");
    let nvm_root = nvm_fixture(&root, &["v9.11.2", "v24.17.0"], Some("22"));

    assert_eq!(
        nvm_bin_dirs(&nvm_root),
        vec![
            nvm_root.join("versions/node/v24.17.0/bin"),
            nvm_root.join("versions/node/v9.11.2/bin"),
        ],
        "v24 outranks v9 numerically, not as a string"
    );
    fs::remove_dir_all(&root).ok();
}

#[test]
fn nvm_bin_dirs_are_empty_without_nvm() {
    let root = temp_root("nvm-absent");
    assert!(nvm_bin_dirs(&root.join(".nvm")).is_empty());
}

// Nothing ever stopped waiting for a child, so one that hung held its stage
// `running` forever. Translation gets hours because a real book takes them;
// the point is that "hours" is a number and "forever" was not.
#[test]
fn every_runner_command_has_a_bounded_timeout() {
    let labelled = |label: &str| RunnerCommand {
        kind: RunnerCommandKind::Process,
        label: label.into(),
        program: PathBuf::from("uv"),
        args: Vec::new(),
        env: Vec::new(),
        cwd: None,
        output_dir: PathBuf::new(),
        attempts: 0,
        accepted_exit_codes: vec![0],
    };
    let translation = runner_command_timeout(&labelled(TRANSLATION_ENGINE_COMMAND_LABEL));
    let conversion = runner_command_timeout(&labelled(ZOTERO_CONVERSION_COMMAND_LABEL));
    let other = runner_command_timeout(&labelled(EPUBCHECK_COMMAND_LABEL));

    assert!(translation > conversion, "translation needs the most room");
    assert!(conversion > other);
    for timeout in [translation, conversion, other] {
        assert!(timeout.as_secs() > 0);
        assert!(
            timeout <= Duration::from_secs(24 * 60 * 60),
            "a day is already generous; anything longer is 'forever' spelled differently"
        );
    }
}

// A stage that stops because a child hung is a different problem from one
// that could not start, and the message has to say which.
#[test]
fn a_timed_out_child_reports_the_timeout_by_name() {
    let mut process = Command::new("sleep");
    process.arg("30");

    let error = crate::command_output_with_timeout(&mut process, Duration::from_millis(150))
        .expect_err("a 30s sleep must not finish inside 150ms");

    assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
    assert!(error.to_string().contains("timed out"), "{error}");
}

// A timeout must fire for a child that hung, and only for that. A pipe holds
// about 64 KB before `write` blocks, so a child that merely says a lot used
// to be indistinguishable from one that stopped responding: nothing read the
// pipe until the child exited, and the child could not exit until something
// read the pipe. The engine prints its entire run report to stdout at once —
// ~474 bytes per chapter, so a book past roughly 140 of them crosses the
// buffer — which made this reachable by a long book rather than by a bug.
#[test]
fn a_talkative_child_is_not_mistaken_for_a_hung_one() {
    use std::time::Instant;

    const VOLUME: usize = 1024 * 1024;
    let mut process = Command::new("sh");
    process.arg("-c").arg(format!(
        "head -c {VOLUME} /dev/zero | tr '\\0' 'x'; \
         head -c {VOLUME} /dev/zero | tr '\\0' 'y' >&2"
    ));

    let started = Instant::now();
    let output = crate::command_output_with_timeout(&mut process, Duration::from_secs(30))
        .expect("a child that exits on its own must not be reported as timed out");

    assert!(output.status.success());
    assert_eq!(output.stdout.len(), VOLUME, "stdout was truncated");
    assert_eq!(output.stderr.len(), VOLUME, "stderr was truncated");
    assert!(
        started.elapsed() < Duration::from_secs(30),
        "the child finished promptly; only a blocked pipe makes this slow"
    );
}

// Every process command the pipeline builds must be a bare name the resolver
// handles or an already-resolved absolute path; a relative multi-component
// program would silently depend on the child's cwd.
#[test]
fn pipeline_process_programs_are_resolvable() {
    for program in ["uv", "node", "java", "python3"] {
        let resolved = resolve_runner_program(Path::new(program));
        assert!(
            resolved.is_absolute() || resolved == PathBuf::from(program),
            "{program} resolved to an unusable relative path: {}",
            display_path(&resolved)
        );
    }
}

struct ArtifactFixtureRunner;

impl PipelineRunner for ArtifactFixtureRunner {
    fn run(&self, _job: &BookPipelineJob, output_dir: &Path) -> Result<RunnerOutput, String> {
        fs::create_dir_all(output_dir).unwrap();
        fs::write(output_dir.join("book.md"), "# Markdown\n").unwrap();
        fs::write(output_dir.join("book.html"), "<h1>HTML</h1>\n").unwrap();
        fs::write(output_dir.join("book.epub"), "epub bytes").unwrap();
        Ok(RunnerOutput {
            log_summary: vec!["fixture runner completed".into()],
            artifacts: scan_artifacts(output_dir)?,
            collection_items: Vec::new(),
            output_dir: Some(output_dir.to_path_buf()),
            current_step: None,
        })
    }
}

struct ConversionFailingRunner;

impl PipelineRunner for ConversionFailingRunner {
    fn run(&self, _job: &BookPipelineJob, _output_dir: &Path) -> Result<RunnerOutput, String> {
        Err("Fake conversion backend failed".into())
    }
}

struct MissingMarkdownRunner;

impl PipelineRunner for MissingMarkdownRunner {
    fn run(&self, _job: &BookPipelineJob, output_dir: &Path) -> Result<RunnerOutput, String> {
        fs::create_dir_all(output_dir).unwrap();
        Ok(RunnerOutput {
            log_summary: vec!["fixture extraction returned no Markdown".into()],
            artifacts: Vec::new(),
            collection_items: Vec::new(),
            output_dir: Some(output_dir.to_path_buf()),
            current_step: None,
        })
    }
}

#[derive(Default)]
struct RecordingNotificationSink {
    events: Mutex<Vec<BookPipelineTerminalEvent>>,
}

impl BookPipelineNotificationSink for RecordingNotificationSink {
    fn deliver(&self, event: &BookPipelineTerminalEvent) -> Result<(), String> {
        self.events.lock().unwrap().push(event.clone());
        Ok(())
    }
}

struct FakeTranslationHandoffRunner;

impl TranslationHandoffRunner for FakeTranslationHandoffRunner {
    fn handoff(
        &self,
        job: &BookPipelineJob,
        artifact_path: Option<&str>,
        repo_root: &Path,
    ) -> Result<TranslationHandoffOutput, String> {
        let markdown = selected_markdown_artifact(job, artifact_path)?;
        let project_root = repo_root
            .join("books")
            .join("local")
            .join("zh-Hans")
            .join("001_fake_handoff");
        let source_path = project_root.join("source").join("source.md");
        let manifest_path = project_root.join("metadata").join("source_manifest.json");
        fs::create_dir_all(source_path.parent().unwrap()).unwrap();
        fs::create_dir_all(manifest_path.parent().unwrap()).unwrap();
        fs::copy(&markdown.path, &source_path).unwrap();
        fs::write(&manifest_path, "{\"schema\":\"fake-source-manifest-v1\"}\n").unwrap();
        Ok(TranslationHandoffOutput {
            log_summary: vec!["Fake translation handoff ready".into()],
            artifacts: vec![
                BookPipelineArtifact {
                    kind: "translation_source".into(),
                    path: display_path(&source_path),
                    sha256: Some(sha256_file(&source_path).unwrap()),
                    zotero_key: markdown.zotero_key.clone(),
                    producer_stage: Some("handoff".into()),
                    ..BookPipelineArtifact::default()
                },
                BookPipelineArtifact {
                    kind: "source_manifest".into(),
                    path: display_path(&manifest_path),
                    sha256: Some(sha256_file(&manifest_path).unwrap()),
                    zotero_key: markdown.zotero_key.clone(),
                    producer_stage: Some("handoff".into()),
                    ..BookPipelineArtifact::default()
                },
            ],
        })
    }

    fn handoff_attachment(
        &self,
        _job: &BookPipelineJob,
        child: &BookPipelineChildJob,
        artifact_path: &str,
        repo_root: &Path,
    ) -> Result<TranslationHandoffOutput, String> {
        assert_eq!(child.source.kind, "zotero_attachment");
        assert!(child
            .artifacts
            .iter()
            .any(|artifact| artifact.kind == "markdown" && artifact.path == artifact_path));
        let attachment_key = child.source.selector.as_deref().unwrap();
        let project_root = repo_root
            .join("books")
            .join("local")
            .join("zh-Hans")
            .join(format!("001_fake_handoff_{attachment_key}"));
        let source_path = project_root.join("source").join("source.md");
        let manifest_path = project_root.join("metadata").join("source_manifest.json");
        fs::create_dir_all(source_path.parent().unwrap()).unwrap();
        fs::create_dir_all(manifest_path.parent().unwrap()).unwrap();
        fs::copy(artifact_path, &source_path).unwrap();
        fs::write(&manifest_path, "{\"schema\":\"fake-source-manifest-v1\"}\n").unwrap();
        let markdown_key = child
            .artifacts
            .iter()
            .find(|artifact| artifact.kind == "markdown" && artifact.path == artifact_path)
            .and_then(|artifact| artifact.zotero_key.clone());
        Ok(TranslationHandoffOutput {
            log_summary: vec!["Fake attachment translation handoff ready".into()],
            artifacts: vec![
                BookPipelineArtifact {
                    kind: "translation_source".into(),
                    path: display_path(&source_path),
                    sha256: Some(sha256_file(&source_path).unwrap()),
                    zotero_key: markdown_key.clone(),
                    producer_stage: Some("handoff".into()),
                    ..BookPipelineArtifact::default()
                },
                BookPipelineArtifact {
                    kind: "source_manifest".into(),
                    path: display_path(&manifest_path),
                    sha256: Some(sha256_file(&manifest_path).unwrap()),
                    zotero_key: markdown_key,
                    producer_stage: Some("handoff".into()),
                    ..BookPipelineArtifact::default()
                },
            ],
        })
    }
}

struct FailingTranslationHandoffRunner;

impl TranslationHandoffRunner for FailingTranslationHandoffRunner {
    fn handoff(
        &self,
        _job: &BookPipelineJob,
        _artifact_path: Option<&str>,
        _repo_root: &Path,
    ) -> Result<TranslationHandoffOutput, String> {
        Err("Fake translation handoff failed".into())
    }
}

struct SecretFailingExecutor;

impl RunnerCommandExecutor for SecretFailingExecutor {
    fn execute(&self, _command: &RunnerCommand) -> Result<RunnerCommandResult, String> {
        Err("ZOTERO_API_KEY=supersecret token=abc Authorization: bearer nope".into())
    }
}

struct SecretLoggingExecutor;

impl RunnerCommandExecutor for SecretLoggingExecutor {
    fn execute(&self, command: &RunnerCommand) -> Result<RunnerCommandResult, String> {
        fs::create_dir_all(&command.output_dir).unwrap();
        fs::write(command.output_dir.join("book.md"), "# Markdown\n").unwrap();
        Ok(RunnerCommandResult {
            stdout: "token=abc".into(),
            stderr: "Authorization: bearer nope".into(),
            log_summary: vec![
                "ZOTERO_API_KEY=supersecret".into(),
                ".env content was not read".into(),
            ],
        })
    }
}

struct LocalPdfFixtureExecutor;

impl RunnerCommandExecutor for LocalPdfFixtureExecutor {
    fn execute(&self, command: &RunnerCommand) -> Result<RunnerCommandResult, String> {
        assert_eq!(command.kind, RunnerCommandKind::Process);
        assert_eq!(command.label, "local PDF conversion wrapper");
        assert!(has_arg_pair(
            &command.args,
            "--output-dir",
            &display_path(&command.output_dir)
        ));
        let book_dir = command.output_dir.join("Sample Book");
        fs::create_dir_all(&book_dir).unwrap();
        fs::write(book_dir.join("sample.md"), "# Markdown\n").unwrap();
        fs::write(book_dir.join("sample.html"), "<h1>HTML</h1>\n").unwrap();
        fs::write(book_dir.join("sample.epub"), "epub bytes").unwrap();
        fs::write(book_dir.join("_state.json"), "{\"status\":\"done\"}\n").unwrap();
        fs::write(book_dir.join("pages.jsonl"), "{\"page\":1}\n").unwrap();
        fs::write(book_dir.join("search.index"), "term -> page 1\n").unwrap();
        Ok(RunnerCommandResult {
            stdout: "DONE: sample.pdf -> sample.html".into(),
            stderr: String::new(),
            log_summary: vec!["Local PDF fixture wrapper completed".into()],
        })
    }
}

// Shaped like the real wrapper output, page-break div included, so the handoff
// is checked against what pdf_to_html_paddleocr.py actually assembles.
const PADDLE_WRAPPER_MARKDOWN: &str =
    "# Sample Book\n\n\n<div class=\"page-break\">— Page 1 —</div>\n\nChapter One\n";

/// Mirrors the on-disk layout of `packages/ocr/scripts/pdf_to_html_paddleocr.py`.
struct PaddleWrapperLayoutExecutor;

impl RunnerCommandExecutor for PaddleWrapperLayoutExecutor {
    fn execute(&self, command: &RunnerCommand) -> Result<RunnerCommandResult, String> {
        assert_eq!(command.kind, RunnerCommandKind::Process);
        assert_eq!(command.label, "local PDF conversion wrapper");
        let book_dir = command.output_dir.join("Sample_Book");
        let assets_dir = book_dir.join("Sample_Book_assets");
        fs::create_dir_all(&assets_dir).unwrap();
        fs::write(book_dir.join("Sample_Book.md"), PADDLE_WRAPPER_MARKDOWN).unwrap();
        fs::write(book_dir.join("Sample_Book.html"), "<h1>Sample Book</h1>\n").unwrap();
        fs::write(
            book_dir.join("_state.json"),
            "{\"markdown_path\":\"Sample_Book.md\"}\n",
        )
        .unwrap();
        fs::write(assets_dir.join("img_1.png"), "png bytes").unwrap();
        let chunks = command
            .output_dir
            .join(".temp")
            .join("Sample_Book")
            .join("chunks");
        fs::create_dir_all(&chunks).unwrap();
        fs::write(chunks.join("pages-0001-0002.jsonl"), "{\"page\":1}\n").unwrap();
        Ok(RunnerCommandResult {
            stdout: String::new(),
            stderr: String::new(),
            log_summary: vec!["Paddle wrapper completed".into()],
        })
    }
}

/// The same layout as `MultiBookLayoutExecutor` for a folder holding one book.
struct SingleBookLayoutExecutor;

impl RunnerCommandExecutor for SingleBookLayoutExecutor {
    fn execute(&self, command: &RunnerCommand) -> Result<RunnerCommandResult, String> {
        assert_eq!(command.label, "local PDF conversion wrapper");
        let book_dir = command.output_dir.join("Sample_Book");
        fs::create_dir_all(&book_dir).unwrap();
        fs::write(book_dir.join("Sample_Book.md"), "# Sample Book\n\nBody\n").unwrap();
        fs::write(book_dir.join("Sample_Book.html"), "<h1>Sample Book</h1>\n").unwrap();
        Ok(RunnerCommandResult {
            stdout: String::new(),
            stderr: String::new(),
            log_summary: vec!["Wrapper converted one book".into()],
        })
    }
}

const MULTI_BOOK_TITLES: [&str; 3] = ["Alpha_Book", "Beta_Book", "Gamma_Book"];

/// Converts a folder into three books, the way the wrapper does: one output
/// directory per book, each holding its own cleaned Markdown.
struct MultiBookLayoutExecutor;

impl RunnerCommandExecutor for MultiBookLayoutExecutor {
    fn execute(&self, command: &RunnerCommand) -> Result<RunnerCommandResult, String> {
        assert_eq!(command.label, "local PDF conversion wrapper");
        for title in MULTI_BOOK_TITLES {
            let book_dir = command.output_dir.join(title);
            fs::create_dir_all(&book_dir).unwrap();
            fs::write(
                book_dir.join(format!("{title}.md")),
                format!("# {title}\n\nBody of {title}\n"),
            )
            .unwrap();
            fs::write(
                book_dir.join(format!("{title}.html")),
                format!("<h1>{title}</h1>\n"),
            )
            .unwrap();
        }
        Ok(RunnerCommandResult {
            stdout: String::new(),
            stderr: String::new(),
            log_summary: vec!["Wrapper converted three books".into()],
        })
    }
}

fn run_multi_book_job(root: &Path) -> BookPipelineJob {
    let input = root.join("input");
    fs::create_dir_all(&input).unwrap();
    for title in MULTI_BOOK_TITLES {
        fs::write(input.join(format!("{title}.pdf")), "%PDF fixture").unwrap();
    }
    let repo = handoff_repo_fixture(root);
    let wrapper_root = fake_wrapper_root(root);
    let store = BookPipelineStore::for_test(root);
    let job = queue_job(
        &store,
        local_pdf_source(&input),
        MODE_CONVERT_THEN_TRANSLATE.into(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();
    run_job_with_handoff(
        &store,
        &CommandPipelineRunner::with_book_ocr_conversion_root(
            MultiBookLayoutExecutor,
            wrapper_root,
        ),
        &LocalProjectHandoffRunner,
        &job.id,
        Some(&repo),
    )
    .unwrap()
}

#[test]
fn every_converted_book_reaches_the_translation_track() {
    let root = temp_root("multi-book-handoff");

    let handed_off = run_multi_book_job(&root);

    assert_eq!(handed_off.status, STATUS_READY);
    assert_eq!(
        handed_off.children.len(),
        3,
        "each converted book needs its own child"
    );
    assert_eq!(
        handed_off.current_stage_id, "children",
        "a multi-book job must aggregate its children, not mirror the first"
    );
    let project_roots = handed_off
        .children
        .iter()
        .filter_map(|child| child.local_project_root.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        project_roots.len(),
        3,
        "every book must reach a local project, got {project_roots:?}"
    );

    // The decisive check: three distinct projects, each carrying its own book.
    let mut bodies = project_roots
        .iter()
        .map(|project| {
            fs::read_to_string(Path::new(project).join("source").join("source.md")).unwrap()
        })
        .collect::<Vec<_>>();
    bodies.sort();
    for title in MULTI_BOOK_TITLES {
        assert!(
            bodies
                .iter()
                .any(|body| body.contains(&format!("Body of {title}"))),
            "{title} never reached translation: {bodies:?}"
        );
    }
    assert_eq!(
        bodies.len(),
        bodies
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        "two projects carry the same book"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn each_book_project_is_named_after_its_own_book() {
    let root = temp_root("multi-book-naming");

    let handed_off = run_multi_book_job(&root);

    let names = handed_off
        .children
        .iter()
        .filter_map(|child| child.local_project_root.as_deref())
        .filter_map(|project| Path::new(project).file_name())
        .filter_map(|name| name.to_str())
        .map(str::to_string)
        .collect::<Vec<_>>();
    for title in MULTI_BOOK_TITLES {
        assert!(
            names.iter().any(|name| name.contains(title)),
            "no project named after {title}: {names:?}"
        );
    }
    assert_eq!(names.len(), 3, "{names:?}");
    let _ = fs::remove_dir_all(root);
}

/// Fails the first book's handoff and lets the rest through.
struct FirstBookHandoffFailingRunner;

impl TranslationHandoffRunner for FirstBookHandoffFailingRunner {
    fn handoff(
        &self,
        job: &BookPipelineJob,
        artifact_path: Option<&str>,
        repo_root: &Path,
    ) -> Result<TranslationHandoffOutput, String> {
        if artifact_path.is_some_and(|path| path.contains(MULTI_BOOK_TITLES[0])) {
            return Err("Fixture handoff failure".into());
        }
        LocalProjectHandoffRunner.handoff(job, artifact_path, repo_root)
    }
}

#[test]
fn one_failed_handoff_does_not_strand_the_remaining_books() {
    let root = temp_root("multi-book-partial-failure");
    let input = root.join("input");
    fs::create_dir_all(&input).unwrap();
    for title in MULTI_BOOK_TITLES {
        fs::write(input.join(format!("{title}.pdf")), "%PDF fixture").unwrap();
    }
    let repo = handoff_repo_fixture(&root);
    let wrapper_root = fake_wrapper_root(&root);
    let store = BookPipelineStore::for_test(&root);
    let job = queue_job(
        &store,
        local_pdf_source(&input),
        MODE_CONVERT_THEN_TRANSLATE.into(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();

    let finished = run_job_with_handoff(
        &store,
        &CommandPipelineRunner::with_book_ocr_conversion_root(
            MultiBookLayoutExecutor,
            wrapper_root,
        ),
        &FirstBookHandoffFailingRunner,
        &job.id,
        Some(&repo),
    )
    .unwrap();

    // The two books whose handoff succeeded still reached a local project.
    let projects = finished
        .children
        .iter()
        .filter(|child| child.local_project_root.is_some())
        .count();
    assert_eq!(projects, 2, "a failure stranded the books after it");
    // And the job still reports the failure rather than hiding it.
    assert_eq!(finished.current_step, "Translation handoff failed");
    // The parent aggregates over every book instead of mirroring the first, so
    // one book's failure stays visible however the children happen to be
    // ordered, and the job cannot report completion while siblings are pending.
    assert_eq!(finished.current_stage_id, "children");
    assert_eq!(finished.summary.failed, 1, "{:?}", finished.summary);
    assert_eq!(finished.summary.ready, 2, "{:?}", finished.summary);
    assert!(finished
        .last_error
        .as_deref()
        .is_some_and(|error| error.contains("Fixture handoff failure")));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_single_book_folder_still_produces_one_child() {
    let root = temp_root("single-book-handoff");
    let input = root.join("input");
    fs::create_dir_all(&input).unwrap();
    fs::write(input.join("Sample Book.pdf"), "%PDF fixture").unwrap();
    let repo = handoff_repo_fixture(&root);
    let wrapper_root = fake_wrapper_root(&root);
    let store = BookPipelineStore::for_test(&root);
    let job = queue_job(
        &store,
        local_pdf_source(&input),
        MODE_CONVERT_THEN_TRANSLATE.into(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();

    let handed_off = run_job_with_handoff(
        &store,
        &CommandPipelineRunner::with_book_ocr_conversion_root(
            SingleBookLayoutExecutor,
            wrapper_root,
        ),
        &LocalProjectHandoffRunner,
        &job.id,
        Some(&repo),
    )
    .unwrap();

    assert_eq!(handed_off.status, STATUS_READY);
    assert_eq!(handed_off.children.len(), 1, "no split for a single book");
    let _ = fs::remove_dir_all(root);
}

struct LocalPdfFailingExecutor;

impl RunnerCommandExecutor for LocalPdfFailingExecutor {
    fn execute(&self, command: &RunnerCommand) -> Result<RunnerCommandResult, String> {
        assert_eq!(command.kind, RunnerCommandKind::Process);
        Err("Local PDF fixture wrapper failed".into())
    }
}

struct ZoteroDiscoveryExecutor;

impl RunnerCommandExecutor for ZoteroDiscoveryExecutor {
    fn execute(&self, command: &RunnerCommand) -> Result<RunnerCommandResult, String> {
        assert_eq!(command.kind, RunnerCommandKind::Process);
        assert_eq!(command.label, "Zotero discovery dry-run");
        assert!(command.args.iter().any(|arg| arg == "--dry-run"));
        assert!(has_arg_pair(&command.args, "--limit", "5"));
        assert!(has_arg_pair(&command.args, "--parent-item-type", "book"));
        Ok(RunnerCommandResult {
            stdout: String::new(),
            stderr: [
                "12:00:00 INFO PLAN DIRECT1 route=pdf-text pages=8 selected=8 parent_type=book sampled_chars=2000 title=Born Digital Book",
                "12:00:01 INFO PLAN SCAN1 route=paddle-ocr pages=240 selected=240 parent_type=book sampled_chars=0 title=Scanned Book",
            ]
            .join("\n"),
            log_summary: vec!["Zotero dry-run completed".into()],
        })
    }
}

struct ZoteroDiscoverySecretFailingExecutor;

impl RunnerCommandExecutor for ZoteroDiscoverySecretFailingExecutor {
    fn execute(&self, _command: &RunnerCommand) -> Result<RunnerCommandResult, String> {
        Err("Zotero discovery failed with ZOTERO_API_KEY=secret".into())
    }
}

struct ZoteroRoutePreviewExecutor;

impl RunnerCommandExecutor for ZoteroRoutePreviewExecutor {
    fn execute(&self, command: &RunnerCommand) -> Result<RunnerCommandResult, String> {
        assert_eq!(command.label, "Zotero discovery dry-run");
        assert!(command.args.iter().any(|arg| arg == "--dry-run"));
        Ok(RunnerCommandResult {
            stdout: String::new(),
            stderr: [
                "12:00:00 INFO PLAN DIRECT route=pdf-text pages=8 selected=8 parent_type=book sampled_chars=2000 title=Direct Text",
                "12:00:01 INFO PLAN SCAN route=paddle-ocr pages=240 selected=240 parent_type=book sampled_chars=0 title=Scanned PDF",
                "12:00:02 INFO PLAN MINERU route=mineru pages=32 selected=32 parent_type=journalArticle sampled_chars=0 title=MinerU Candidate",
                "12:00:03 INFO PLAN DIRTY route=needs-mineru pages=12 selected=12 parent_type=book sampled_chars=600 title=Dirty Text Layer",
                "12:00:04 INFO SKIP completed DONE Already Converted",
            ]
            .join("\n"),
            log_summary: vec!["Zotero route dry-run completed".into()],
        })
    }
}

struct ZoteroFingerprintPreviewExecutor;

impl RunnerCommandExecutor for ZoteroFingerprintPreviewExecutor {
    fn execute(&self, command: &RunnerCommand) -> Result<RunnerCommandResult, String> {
        assert_eq!(command.label, "Zotero discovery dry-run");
        Ok(RunnerCommandResult {
            stdout: String::new(),
            stderr: [
                "12:00:00 INFO SKIP completed CURRENT Current Title source_md5=aaa111 output_path=/tmp/current.md zotero_attachment_key=MDOLD",
                "12:00:01 INFO REBUILD completed MISSING because uploaded Zotero attachment is missing",
                "12:00:02 INFO PLAN MISSING route=pdf-text pages=10 selected=10 parent_type=book sampled_chars=1200 title=Missing Upload source_md5=aaa111",
                "12:00:03 INFO PLAN CHANGED route=pdf-text pages=12 selected=12 parent_type=book sampled_chars=1400 title=Changed Source source_md5=bbb222",
                "12:00:04 INFO PLAN DIRTY route=needs-mineru pages=8 selected=8 parent_type=book sampled_chars=500 title=Dirty Blocked source_md5=ccc333",
            ]
            .join("\n"),
            log_summary: vec!["Zotero fingerprint dry-run completed".into()],
        })
    }
}

fn fixture_item_index_profile_result() -> RunnerCommandResult {
    RunnerCommandResult {
        stdout: serde_json::json!({
            "embeddingProfileId": "fixture-embedding:3",
        })
        .to_string(),
        stderr: String::new(),
        log_summary: vec!["Zotero item index profile fixture completed".into()],
    }
}

struct ZoteroConversionExecutor;

impl RunnerCommandExecutor for ZoteroConversionExecutor {
    fn execute(&self, command: &RunnerCommand) -> Result<RunnerCommandResult, String> {
        assert_eq!(command.kind, RunnerCommandKind::Process);
        match command.label.as_str() {
            ITEM_INDEX_PROFILE_COMMAND_LABEL => Ok(fixture_item_index_profile_result()),
            "Zotero conversion worker" => {
                assert!(has_arg_pair(&command.args, "--attachment-key", "DIRECT"));
                assert!(command.args.iter().any(|arg| arg == "--force-text"));
                assert!(has_env_pair(
                    &command.env,
                    "OCR_OUTPUT_ROOT",
                    &display_path(&command.output_dir)
                ));
                let staging = command
                    .output_dir
                    .join(".state")
                    .join("staging")
                    .join("DIRECT");
                fs::create_dir_all(&staging).unwrap();
                fs::write(
                    staging.join("direct.md"),
                    "---\nparent_item_key: \"PARENT123\"\n---\n\n# Direct Markdown\n",
                )
                .unwrap();
                fs::write(staging.join("direct.jsonl"), "{\"page\":1}\n").unwrap();
                Ok(RunnerCommandResult {
                    stdout: "Uploaded direct.md to Zotero attachment MDKEY123 status=completed"
                        .into(),
                    stderr: String::new(),
                    log_summary: vec!["Zotero conversion fixture completed".into()],
                })
            }
            "Zotero item-scoped full-text index" => {
                let markdown_sha256 = command_arg_value(command, "--sha256").unwrap();
                Ok(RunnerCommandResult {
                    stdout: serde_json::json!({
                        "parentItemKey": "PARENT123",
                        "sourceSha256": markdown_sha256,
                        "chunkCount": 1,
                        "indexContractVersion": ITEM_INDEX_CONTRACT_VERSION,
                        "chunkContractVersion": CHUNK_CONTRACT_VERSION,
                        "embeddingProfileId": "fixture-embedding:3",
                        "completedAt": "2026-07-15T12:00:00Z",
                        "reused": false,
                    })
                    .to_string(),
                    stderr: String::new(),
                    log_summary: vec!["Zotero item index fixture completed".into()],
                })
            }
            other => panic!("unexpected command: {other}"),
        }
    }
}

struct ZoteroExtractIndexExecutor {
    command_labels: Mutex<Vec<String>>,
    fail_index_once: Mutex<bool>,
    omit_markdown_attachment_key: bool,
}

impl ZoteroExtractIndexExecutor {
    fn succeeding() -> Self {
        Self {
            command_labels: Mutex::new(Vec::new()),
            fail_index_once: Mutex::new(false),
            omit_markdown_attachment_key: false,
        }
    }

    fn failing_index_once() -> Self {
        Self {
            command_labels: Mutex::new(Vec::new()),
            fail_index_once: Mutex::new(true),
            omit_markdown_attachment_key: false,
        }
    }

    fn missing_markdown_attachment_key() -> Self {
        Self {
            command_labels: Mutex::new(Vec::new()),
            fail_index_once: Mutex::new(false),
            omit_markdown_attachment_key: true,
        }
    }

    fn command_labels(&self) -> Vec<String> {
        self.command_labels.lock().unwrap().clone()
    }
}

impl RunnerCommandExecutor for ZoteroExtractIndexExecutor {
    fn execute(&self, command: &RunnerCommand) -> Result<RunnerCommandResult, String> {
        self.command_labels
            .lock()
            .unwrap()
            .push(command.label.clone());
        match command.label.as_str() {
            ITEM_INDEX_PROFILE_COMMAND_LABEL => Ok(fixture_item_index_profile_result()),
            "Zotero conversion worker" => {
                let staging = command
                    .output_dir
                    .join(".state")
                    .join("staging")
                    .join("DIRECT");
                fs::create_dir_all(&staging).unwrap();
                fs::write(
                    staging.join("direct.md"),
                    "---\nparent_item_key: \"PARENT123\"\nsource_pdf_key: \"DIRECT\"\n---\n\n# Direct Markdown\n",
                )
                .unwrap();
                let stdout = if self.omit_markdown_attachment_key {
                    "Completed direct.md without upload evidence status=completed"
                } else {
                    "Uploaded direct.md to Zotero attachment MDKEY123 status=completed"
                };
                Ok(RunnerCommandResult {
                    stdout: stdout.into(),
                    stderr: String::new(),
                    log_summary: vec!["Zotero conversion fixture completed".into()],
                })
            }
            "Zotero item-scoped full-text index" => {
                let mut fail_index_once = self.fail_index_once.lock().unwrap();
                if *fail_index_once {
                    *fail_index_once = false;
                    return Err("fixture index backend unavailable".into());
                }
                assert!(has_arg_pair(
                    &command.args,
                    "--parent-item-key",
                    "PARENT123"
                ));
                assert!(has_arg_pair(
                    &command.args,
                    "--chunk-contract-version",
                    "zfulltext-chunk-v2"
                ));
                assert!(has_arg_pair(
                    &command.args,
                    "--embedding-profile-id",
                    "fixture-embedding:3"
                ));
                let markdown_path = command_arg_value(command, "--markdown").unwrap();
                let markdown_sha256 = command_arg_value(command, "--sha256").unwrap();
                assert!(Path::new(markdown_path).is_file());
                assert_eq!(
                    sha256_file(Path::new(markdown_path)).unwrap(),
                    markdown_sha256
                );
                Ok(RunnerCommandResult {
                    stdout: serde_json::json!({
                        "parentItemKey": "PARENT123",
                        "sourceSha256": markdown_sha256,
                        "chunkCount": 1,
                        "indexContractVersion": "zfulltext-item-index-v1",
                        "chunkContractVersion": "zfulltext-chunk-v2",
                        "embeddingProfileId": "fixture-embedding:3",
                        "completedAt": "2026-07-15T12:00:00Z",
                        "reused": false,
                    })
                    .to_string(),
                    stderr: String::new(),
                    log_summary: vec!["Zotero item index fixture completed".into()],
                })
            }
            other => panic!("unexpected command: {other}"),
        }
    }
}

struct ZoteroConversionFailingExecutor;

impl RunnerCommandExecutor for ZoteroConversionFailingExecutor {
    fn execute(&self, command: &RunnerCommand) -> Result<RunnerCommandResult, String> {
        assert_eq!(command.label, "Zotero conversion worker");
        Err("Zotero conversion fixture failed: diagnosis preserved".into())
    }
}

struct MineruFixtureExecutor;

impl RunnerCommandExecutor for MineruFixtureExecutor {
    fn execute(&self, command: &RunnerCommand) -> Result<RunnerCommandResult, String> {
        assert_eq!(command.kind, RunnerCommandKind::Process);
        match command.label.as_str() {
            ITEM_INDEX_PROFILE_COMMAND_LABEL => Ok(fixture_item_index_profile_result()),
            ZOTERO_CONVERSION_COMMAND_LABEL => {
                assert!(has_arg_pair(&command.args, "--attachment-key", "MINERU"));
                assert!(command.args.iter().any(|arg| arg == "--force-mineru"));
                fs::create_dir_all(&command.output_dir).unwrap();
                fs::write(
                    command.output_dir.join("mineru.md"),
                    "---\nparent_item_key: \"PARENTMINERU\"\n---\n\n# MinerU Markdown\n",
                )
                .unwrap();
                fs::write(
                    command.output_dir.join("mineru.json"),
                    "{\"engine\":\"mineru\"}\n",
                )
                .unwrap();
                Ok(RunnerCommandResult {
                    stdout: "Uploaded mineru.md to Zotero attachment MINERUMD".into(),
                    stderr: String::new(),
                    log_summary: vec!["MinerU fixture completed".into()],
                })
            }
            "Zotero item-scoped full-text index" => {
                let markdown_sha256 = command_arg_value(command, "--sha256").unwrap();
                Ok(RunnerCommandResult {
                    stdout: serde_json::json!({
                        "parentItemKey": "PARENTMINERU",
                        "sourceSha256": markdown_sha256,
                        "chunkCount": 1,
                        "indexContractVersion": ITEM_INDEX_CONTRACT_VERSION,
                        "chunkContractVersion": CHUNK_CONTRACT_VERSION,
                        "embeddingProfileId": "fixture-embedding:3",
                        "completedAt": "2026-07-15T12:00:00Z",
                        "reused": false,
                    })
                    .to_string(),
                    stderr: String::new(),
                    log_summary: vec!["Zotero item index fixture completed".into()],
                })
            }
            other => panic!("unexpected command: {other}"),
        }
    }
}

struct ExternalAdapterFixtureExecutor;

impl RunnerCommandExecutor for ExternalAdapterFixtureExecutor {
    fn execute(&self, command: &RunnerCommand) -> Result<RunnerCommandResult, String> {
        assert_eq!(command.kind, RunnerCommandKind::Process);
        assert_eq!(command.label, "external Book Pipeline adapter");
        assert!(has_arg_pair(
            &command.args,
            "--output-dir",
            &display_path(&command.output_dir)
        ));
        fs::create_dir_all(&command.output_dir).unwrap();
        fs::write(
            command.output_dir.join("adapter.md"),
            "# Adapter Markdown\n",
        )
        .unwrap();
        fs::write(
            command.output_dir.join("adapter.html"),
            "<h1>Adapter</h1>\n",
        )
        .unwrap();
        Ok(RunnerCommandResult {
            stdout: "adapter completed".into(),
            stderr: String::new(),
            log_summary: vec!["External adapter fixture completed".into()],
        })
    }
}

struct TranslationEngineFixtureExecutor {
    fail_once: Mutex<Option<String>>,
    failure_code: String,
    requested_units: Mutex<Vec<Vec<String>>>,
    expected_second_pass_enabled: bool,
    expected_text_cleanup: bool,
    expected_custom_instructions: Option<BookPipelineCustomInstructions>,
    merge_translation_paragraphs: bool,
    // Emitted on every completed unit, so a test can drive the runner's
    // handling of a glossary warning without a real model to disobey.
    glossary_violations: Vec<(String, String)>,
}

impl TranslationEngineFixtureExecutor {
    fn succeeding() -> Self {
        Self {
            fail_once: Mutex::new(None),
            failure_code: "translation_structure_invalid".into(),
            requested_units: Mutex::new(Vec::new()),
            expected_second_pass_enabled: false,
            expected_text_cleanup: false,
            expected_custom_instructions: None,
            merge_translation_paragraphs: false,
            glossary_violations: Vec::new(),
        }
    }

    fn with_second_pass_enabled() -> Self {
        Self {
            fail_once: Mutex::new(None),
            failure_code: "translation_structure_invalid".into(),
            requested_units: Mutex::new(Vec::new()),
            expected_second_pass_enabled: true,
            expected_text_cleanup: false,
            expected_custom_instructions: None,
            merge_translation_paragraphs: false,
            glossary_violations: Vec::new(),
        }
    }

    fn with_text_cleanup() -> Self {
        Self {
            fail_once: Mutex::new(None),
            failure_code: "translation_structure_invalid".into(),
            requested_units: Mutex::new(Vec::new()),
            expected_second_pass_enabled: false,
            expected_text_cleanup: true,
            expected_custom_instructions: None,
            merge_translation_paragraphs: false,
            glossary_violations: Vec::new(),
        }
    }

    fn with_custom_instructions(custom_instructions: BookPipelineCustomInstructions) -> Self {
        Self {
            fail_once: Mutex::new(None),
            failure_code: "translation_structure_invalid".into(),
            requested_units: Mutex::new(Vec::new()),
            expected_second_pass_enabled: true,
            expected_text_cleanup: false,
            expected_custom_instructions: Some(custom_instructions),
            merge_translation_paragraphs: false,
            glossary_violations: Vec::new(),
        }
    }

    fn failing_once(unit_id: &str) -> Self {
        Self {
            fail_once: Mutex::new(Some(unit_id.into())),
            failure_code: "translation_structure_invalid".into(),
            requested_units: Mutex::new(Vec::new()),
            expected_second_pass_enabled: false,
            expected_text_cleanup: false,
            expected_custom_instructions: None,
            merge_translation_paragraphs: false,
            glossary_violations: Vec::new(),
        }
    }

    fn failing_once_with_code(unit_id: &str, failure_code: &str) -> Self {
        Self {
            failure_code: failure_code.into(),
            ..Self::failing_once(unit_id)
        }
    }

    fn with_paragraph_mismatch() -> Self {
        Self {
            fail_once: Mutex::new(None),
            failure_code: "translation_structure_invalid".into(),
            requested_units: Mutex::new(Vec::new()),
            expected_second_pass_enabled: false,
            expected_text_cleanup: false,
            expected_custom_instructions: None,
            merge_translation_paragraphs: true,
            glossary_violations: Vec::new(),
        }
    }

    fn reporting_glossary_violations(violations: &[(&str, &str)]) -> Self {
        Self {
            glossary_violations: violations
                .iter()
                .map(|(source, translation)| ((*source).to_string(), (*translation).to_string()))
                .collect(),
            ..Self::succeeding()
        }
    }

    fn requested_units(&self) -> Vec<Vec<String>> {
        self.requested_units.lock().unwrap().clone()
    }
}

impl RunnerCommandExecutor for TranslationEngineFixtureExecutor {
    fn execute(&self, command: &RunnerCommand) -> Result<RunnerCommandResult, String> {
        assert_eq!(command.kind, RunnerCommandKind::Process);
        assert_eq!(command.label, TRANSLATION_ENGINE_COMMAND_LABEL);
        assert_eq!(command.program, PathBuf::from("uv"));
        let repo_root = local_reading_repo_root().unwrap();
        assert_eq!(command.cwd.as_deref(), Some(repo_root.as_path()));
        assert_eq!(command.accepted_exit_codes, vec![0, 1]);
        let manifest_path = PathBuf::from(&command.args[5]);
        assert_eq!(
            command.args,
            vec![
                "run".to_string(),
                "--package".to_string(),
                "translation-engine".to_string(),
                "translation-engine".to_string(),
                "--manifest".to_string(),
                display_path(&manifest_path),
            ]
        );
        let manifest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&manifest_path).unwrap()).unwrap();
        assert_eq!(manifest["schema"], TRANSLATION_ENGINE_RUN_SCHEMA);
        assert_eq!(manifest["projectRoot"], display_path(&command.output_dir));
        assert_eq!(manifest["sourceMapPath"], "metadata/source_map.json");
        assert_eq!(manifest["sourceLanguage"], "auto");
        assert_eq!(manifest["targetLanguage"], "zh-Hans");
        assert_eq!(manifest["providerProfileId"], "fake-provider-profile");
        assert_eq!(manifest["providerConfigId"], "fake-provider-config");
        match &self.expected_custom_instructions {
            Some(custom_instructions) => assert_eq!(
                manifest["customInstructions"],
                serde_json::to_value(custom_instructions).unwrap()
            ),
            None => assert!(manifest.get("customInstructions").is_none()),
        }
        assert_eq!(
            manifest["secondPassEnabled"],
            self.expected_second_pass_enabled
        );
        assert_eq!(manifest["textCleanup"], self.expected_text_cleanup);
        assert_eq!(
            manifest["translationPolicyVersion"],
            TRANSLATION_POLICY_VERSION
        );
        assert_eq!(TRANSLATION_POLICY_VERSION, "translation-policy-v10");
        assert_eq!(manifest["maxTokens"], TRANSLATION_ENGINE_MAX_TOKENS);
        assert_eq!(
            manifest["placeholderRetries"],
            TRANSLATION_ENGINE_PLACEHOLDER_RETRIES
        );

        let mut requested = Vec::new();
        for unit in manifest["units"].as_array().unwrap() {
            let task_path = command
                .output_dir
                .join(unit["taskManifestPath"].as_str().unwrap());
            let task: serde_json::Value =
                serde_json::from_str(&fs::read_to_string(task_path).unwrap()).unwrap();
            requested.push((
                task["chapterId"].as_str().unwrap().to_string(),
                task["sourceChapterPath"].as_str().unwrap().to_string(),
            ));
        }
        self.requested_units.lock().unwrap().push(
            requested
                .iter()
                .map(|(unit_id, _)| unit_id.clone())
                .collect(),
        );
        let fail_unit = self.fail_once.lock().unwrap().take();
        let mut reports = Vec::new();
        for (unit_id, source_chapter_path) in requested {
            let failed = fail_unit.as_deref() == Some(unit_id.as_str());
            let relative = if failed {
                format!("chapters/translated/.partial/{unit_id}.degraded.md")
            } else {
                format!("chapters/translated/{unit_id}.md")
            };
            let path = command.output_dir.join(&relative);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            let source = fs::read_to_string(command.output_dir.join(source_chapter_path)).unwrap();
            let mut translation = fixture_translation(&source, &unit_id);
            if self.merge_translation_paragraphs {
                translation = translation.replace("\n\n", "\n");
            }
            fs::write(&path, translation).unwrap();
            if !failed {
                let _ = fs::remove_file(
                    command
                        .output_dir
                        .join("chapters")
                        .join("translated")
                        .join(".partial")
                        .join(format!("{unit_id}.degraded.md")),
                );
            }
            let artifact = serde_json::json!({
                "kind": if failed {
                    "chapter_translation_degraded"
                } else {
                    "chapter_translation"
                },
                "path": relative,
                "sha256": sha256_file(&path).unwrap(),
                "complete": !failed,
            });
            reports.push(if failed {
                serde_json::json!({
                    "unitId": unit_id,
                    "status": "failed",
                    "artifact": artifact,
                    "error": {"code": self.failure_code.as_str(), "retryable": true},
                })
            } else {
                let mut completed = serde_json::json!({
                    "unitId": unit_id,
                    "status": "completed",
                    "artifact": artifact,
                });
                if !self.glossary_violations.is_empty() {
                    completed["glossaryViolations"] = serde_json::Value::Array(
                        self.glossary_violations
                            .iter()
                            .map(|(source, translation)| {
                                serde_json::json!({
                                    "source": source,
                                    "translation": translation,
                                })
                            })
                            .collect(),
                    );
                }
                completed
            });
        }
        let failed = reports
            .iter()
            .filter(|report| report["status"] == "failed")
            .count();
        let total = reports.len();
        let report = serde_json::json!({
            "schema": TRANSLATION_ENGINE_REPORT_SCHEMA,
            "summary": {
                "total": total,
                "completed": total - failed,
                "failed": failed,
            },
            "units": reports,
        });
        Ok(RunnerCommandResult {
            stdout: serde_json::to_string(&report).unwrap(),
            stderr: String::new(),
            log_summary: vec!["Translation engine fixture completed".into()],
        })
    }
}

#[derive(Default)]
struct TranslationSampleFixtureExecutor {
    requests: Mutex<Vec<(String, String)>>,
    // Recorded so a test can assert the sample manifest carries the same
    // translation settings as the full run; without them the preview shows a
    // translation the real run would not produce.
    prompt_inputs: Mutex<Vec<(serde_json::Value, serde_json::Value)>>,
}

impl TranslationSampleFixtureExecutor {
    fn requests(&self) -> Vec<(String, String)> {
        self.requests.lock().unwrap().clone()
    }

    fn prompt_inputs(&self) -> Vec<(serde_json::Value, serde_json::Value)> {
        self.prompt_inputs.lock().unwrap().clone()
    }
}

impl RunnerCommandExecutor for TranslationSampleFixtureExecutor {
    fn execute(&self, command: &RunnerCommand) -> Result<RunnerCommandResult, String> {
        assert_eq!(command.kind, RunnerCommandKind::Process);
        assert_eq!(command.label, TRANSLATION_ENGINE_SAMPLE_COMMAND_LABEL);
        assert_eq!(command.program, PathBuf::from("uv"));
        assert_eq!(command.accepted_exit_codes, vec![0]);
        let manifest_path = PathBuf::from(&command.args[5]);
        assert_eq!(
            command.args,
            vec![
                "run".to_string(),
                "--package".to_string(),
                "translation-engine".to_string(),
                "translation-engine-sample".to_string(),
                "--manifest".to_string(),
                display_path(&manifest_path),
            ]
        );
        let manifest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(manifest_path).unwrap()).unwrap();
        assert_eq!(manifest["schema"], TRANSLATION_ENGINE_SAMPLE_SCHEMA);
        assert_eq!(manifest["sampleCount"], TRANSLATION_SAMPLE_COUNT);
        assert_eq!(
            manifest["characterBudget"],
            TRANSLATION_SAMPLE_CHARACTER_BUDGET
        );
        let profile = manifest["providerProfileId"].as_str().unwrap().to_string();
        let config = manifest["providerConfigId"].as_str().unwrap().to_string();
        self.requests
            .lock()
            .unwrap()
            .push((profile, config.clone()));
        self.prompt_inputs.lock().unwrap().push((
            manifest["textCleanup"].clone(),
            manifest
                .get("customInstructions")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
        ));

        let units = manifest["units"].as_array().unwrap();
        assert_eq!(units.len(), 5);
        let mut chapter_ids = Vec::new();
        for unit in units {
            let task_path = command
                .output_dir
                .join(unit["taskManifestPath"].as_str().unwrap());
            let task: serde_json::Value =
                serde_json::from_str(&fs::read_to_string(task_path).unwrap()).unwrap();
            chapter_ids.push(task["chapterId"].as_str().unwrap().to_string());
        }
        let samples = chapter_ids[1..4]
            .iter()
            .map(|chapter_id| {
                serde_json::json!({
                    "chunkRef": chapter_id,
                    "sourceExcerpt": format!("Source {chapter_id}."),
                    "translatedExcerpt": format!("{config}: Translated {chapter_id}."),
                    "degradation": if chapter_id == "chapter_003" { "aligned" } else { "none" },
                })
            })
            .collect::<Vec<_>>();
        Ok(RunnerCommandResult {
            stdout: serde_json::to_string(&serde_json::json!({
                "schema": TRANSLATION_ENGINE_SAMPLE_REPORT_SCHEMA,
                "samples": samples,
            }))
            .unwrap(),
            stderr: String::new(),
            log_summary: vec!["Translation sample fixture completed".into()],
        })
    }
}

struct ReadingPipelineFixtureExecutor {
    translation: TranslationEngineFixtureExecutor,
    reading_epubcheck_passes: bool,
    digest_epubcheck_passes: bool,
    digest_enabled: bool,
    bilingual_fallback: bool,
    command_labels: Mutex<Vec<String>>,
}

impl ReadingPipelineFixtureExecutor {
    fn passing() -> Self {
        Self {
            translation: TranslationEngineFixtureExecutor::succeeding(),
            reading_epubcheck_passes: true,
            digest_epubcheck_passes: true,
            digest_enabled: false,
            bilingual_fallback: false,
            command_labels: Mutex::new(Vec::new()),
        }
    }

    fn passing_with_digest() -> Self {
        Self {
            translation: TranslationEngineFixtureExecutor::succeeding(),
            reading_epubcheck_passes: true,
            digest_epubcheck_passes: true,
            digest_enabled: true,
            bilingual_fallback: false,
            command_labels: Mutex::new(Vec::new()),
        }
    }

    fn failing_epubcheck() -> Self {
        Self {
            translation: TranslationEngineFixtureExecutor::succeeding(),
            reading_epubcheck_passes: false,
            digest_epubcheck_passes: true,
            digest_enabled: false,
            bilingual_fallback: false,
            command_labels: Mutex::new(Vec::new()),
        }
    }

    fn passing_with_bilingual_fallback() -> Self {
        Self {
            translation: TranslationEngineFixtureExecutor::with_paragraph_mismatch(),
            reading_epubcheck_passes: true,
            digest_epubcheck_passes: true,
            digest_enabled: false,
            bilingual_fallback: true,
            command_labels: Mutex::new(Vec::new()),
        }
    }

    fn failing_digest_epubcheck() -> Self {
        Self {
            translation: TranslationEngineFixtureExecutor::succeeding(),
            reading_epubcheck_passes: true,
            digest_epubcheck_passes: false,
            digest_enabled: true,
            bilingual_fallback: false,
            command_labels: Mutex::new(Vec::new()),
        }
    }

    fn command_labels(&self) -> Vec<String> {
        self.command_labels.lock().unwrap().clone()
    }
}

impl RunnerCommandExecutor for ReadingPipelineFixtureExecutor {
    fn execute(&self, command: &RunnerCommand) -> Result<RunnerCommandResult, String> {
        self.command_labels
            .lock()
            .unwrap()
            .push(command.label.clone());
        match command.label.as_str() {
            TRANSLATION_ENGINE_COMMAND_LABEL => self.translation.execute(command),
            READING_BUILD_COMMAND_LABEL => {
                assert_eq!(command.kind, RunnerCommandKind::Process);
                assert_eq!(command.program, PathBuf::from("node"));
                assert_eq!(command.cwd.as_deref(), Some(command.output_dir.as_path()));
                assert_eq!(command.accepted_exit_codes, vec![0]);
                assert_eq!(command.args.len(), 1);
                assert_eq!(
                    Path::new(&command.args[0])
                        .file_name()
                        .and_then(|name| name.to_str()),
                    Some("build_epub.js")
                );
                assert!(command.output_dir.join("output/reading/book.md").is_file());
                let final_dir = command.output_dir.join("chapters/final");
                let mut final_paths = fs::read_dir(&final_dir)
                    .unwrap()
                    .map(|entry| entry.unwrap().path())
                    .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("md"))
                    .collect::<Vec<_>>();
                final_paths.sort();
                assert!(!final_paths.is_empty());
                let html_dir = command.output_dir.join("output/reading/html");
                fs::create_dir_all(&html_dir).unwrap();
                for path in final_paths {
                    let unit_id = path.file_stem().unwrap().to_string_lossy();
                    fs::write(
                        html_dir.join(format!("{unit_id}.xhtml")),
                        format!("<html><body><p>{unit_id}</p></body></html>\n"),
                    )
                    .unwrap();
                }
                fs::write(
                    command.output_dir.join("output/reading/book.epub"),
                    "canned epub",
                )
                .unwrap();
                Ok(RunnerCommandResult {
                    stdout: "wrote output/reading/book.epub".into(),
                    stderr: String::new(),
                    log_summary: vec!["Reading builder fixture completed".into()],
                })
            }
            BILINGUAL_BUILD_COMMAND_LABEL => {
                assert_eq!(command.kind, RunnerCommandKind::Process);
                assert_eq!(command.program, PathBuf::from("python3"));
                assert_eq!(command.cwd.as_deref(), Some(command.output_dir.as_path()));
                assert_eq!(command.accepted_exit_codes, vec![0]);
                assert_eq!(command.args.len(), 3);
                assert_eq!(
                    Path::new(&command.args[0])
                        .file_name()
                        .and_then(|name| name.to_str()),
                    Some("build_bilingual_epub.py")
                );
                assert_eq!(command.args[1], "--book-root");
                assert_eq!(command.args[2], display_path(&command.output_dir));
                assert!(command
                    .output_dir
                    .join("metadata/source_map.json")
                    .is_file());
                assert!(command
                    .output_dir
                    .join("chapters/src/chapter_001.md")
                    .is_file());
                assert!(command
                    .output_dir
                    .join("chapters/final/chapter_001.md")
                    .is_file());
                let paragraph_count = |path: &Path| {
                    fs::read_to_string(path)
                        .unwrap()
                        .split("\n\n")
                        .filter(|paragraph| !paragraph.trim().is_empty())
                        .count()
                };
                let source_count =
                    paragraph_count(&command.output_dir.join("chapters/src/chapter_001.md"));
                let target_count =
                    paragraph_count(&command.output_dir.join("chapters/final/chapter_001.md"));
                let alignment = if self.bilingual_fallback {
                    assert_ne!(source_count, target_count);
                    "chapter-fallback"
                } else {
                    assert_eq!(source_count, target_count);
                    "paragraph"
                };
                fs::write(
                    command
                        .output_dir
                        .join("output/reading/book_bilingual.epub"),
                    "canned bilingual epub",
                )
                .unwrap();
                Ok(RunnerCommandResult {
                    stdout: format!(
                        "chapter_001: alignment={alignment} source_paragraphs={source_count} target_paragraphs={target_count}\nwrote output/reading/book_bilingual.epub"
                    ),
                    stderr: String::new(),
                    log_summary: vec!["Bilingual builder fixture completed".into()],
                })
            }
            EPUBCHECK_COMMAND_LABEL => {
                assert_eq!(command.kind, RunnerCommandKind::Process);
                assert_eq!(command.program, PathBuf::from("java"));
                assert_eq!(command.cwd.as_deref(), Some(command.output_dir.as_path()));
                assert_eq!(command.accepted_exit_codes, vec![0, 1]);
                assert_eq!(command.args[0], "-jar");
                assert!(command.args[1].ends_with("epubcheck.jar"));
                assert!(Path::new(&command.args[2]).is_file());
                assert_eq!(command.args[3], "--json");
                assert_eq!(command.args[5], "-q");
                let epub_path = PathBuf::from(&command.args[2]);
                let digest_epubcheck = epub_path.file_name().and_then(|name| name.to_str())
                    == Some("book_digest.epub");
                let epubcheck_passes = if digest_epubcheck {
                    self.digest_epubcheck_passes
                } else {
                    self.reading_epubcheck_passes
                };
                let report_path = PathBuf::from(&command.args[4]);
                fs::write(
                    &report_path,
                    serde_json::to_string_pretty(&serde_json::json!({
                        "checker": {
                            "nFatal": 0,
                            "nError": if epubcheck_passes { 0 } else { 1 },
                            "nWarning": if epubcheck_passes { 1 } else { 0 },
                        }
                    }))
                    .unwrap()
                        + "\n",
                )
                .unwrap();
                Ok(RunnerCommandResult {
                    stdout: if epubcheck_passes {
                        "epubcheck: fatal=0, error=0, warning=1".into()
                    } else {
                        "epubcheck: fatal=0, error=1, warning=0".into()
                    },
                    stderr: String::new(),
                    log_summary: vec!["EPUBCheck fixture completed".into()],
                })
            }
            DIGEST_BUILD_COMMAND_LABEL if self.digest_enabled => {
                assert_eq!(command.kind, RunnerCommandKind::Process);
                assert_eq!(command.program, PathBuf::from("uv"));
                assert_eq!(
                    command.cwd.as_deref(),
                    Some(local_reading_repo_root().unwrap().as_path())
                );
                assert_eq!(command.accepted_exit_codes, vec![0]);
                assert_eq!(
                    command.args,
                    vec![
                        "run".to_string(),
                        "--package".to_string(),
                        "digest".to_string(),
                        "python".to_string(),
                        "-m".to_string(),
                        "digest.bibliosmith_digest".to_string(),
                        "--book-root".to_string(),
                        display_path(&command.output_dir),
                    ]
                );
                let config: serde_json::Value = serde_json::from_str(
                    &fs::read_to_string(command.output_dir.join("digest.config.json")).unwrap(),
                )
                .unwrap();
                assert_eq!(config["enabled"], true);
                assert_eq!(config["merge_into_epub"], true);
                assert_eq!(config["source_epub"], "output/reading/book.epub");
                assert_eq!(config["output_epub"], "output/reading/book_digest.epub");
                assert_eq!(config["title"], "Digest Fixture Title");
                assert_eq!(config["language"], "zh-CN");
                fs::create_dir_all(command.output_dir.join("output/reading/digest")).unwrap();
                fs::create_dir_all(command.output_dir.join("qa/digest")).unwrap();
                fs::write(
                    command.output_dir.join("output/reading/book_digest.epub"),
                    "canned digest epub",
                )
                .unwrap();
                fs::write(
                    command
                        .output_dir
                        .join("output/reading/digest/digest.xhtml"),
                    "<html><body>Digest</body></html>\n",
                )
                .unwrap();
                fs::write(
                    command
                        .output_dir
                        .join("output/reading/digest/knowledge_map.svg"),
                    "<svg xmlns=\"http://www.w3.org/2000/svg\"></svg>\n",
                )
                .unwrap();
                fs::write(
                    command
                        .output_dir
                        .join("qa/digest/digest_review_checklist.md"),
                    "# Digest Review\n",
                )
                .unwrap();
                fs::write(
                    command.output_dir.join("qa/digest/digest_report.json"),
                    serde_json::to_string_pretty(&serde_json::json!({
                        "status": "PASS",
                        "merged": true,
                        "source_epub": "output/reading/book.epub",
                        "output_epub": "output/reading/book_digest.epub",
                    }))
                    .unwrap()
                        + "\n",
                )
                .unwrap();
                Ok(RunnerCommandResult {
                    stdout: "digest status=PASS".into(),
                    stderr: String::new(),
                    log_summary: vec!["Digest fixture completed".into()],
                })
            }
            other => panic!("unexpected reading pipeline command {other}"),
        }
    }
}

struct ZoteroBatchFixtureExecutor;

impl RunnerCommandExecutor for ZoteroBatchFixtureExecutor {
    fn execute(&self, command: &RunnerCommand) -> Result<RunnerCommandResult, String> {
        if command.label == ITEM_INDEX_PROFILE_COMMAND_LABEL {
            return Ok(fixture_item_index_profile_result());
        }
        if command.label == ITEM_INDEX_COMMAND_LABEL {
            let parent_item_key = command_arg_value(command, "--parent-item-key").unwrap();
            let markdown_sha256 = command_arg_value(command, "--sha256").unwrap();
            return Ok(RunnerCommandResult {
                stdout: serde_json::json!({
                    "parentItemKey": parent_item_key,
                    "sourceSha256": markdown_sha256,
                    "chunkCount": 1,
                    "indexContractVersion": ITEM_INDEX_CONTRACT_VERSION,
                    "chunkContractVersion": CHUNK_CONTRACT_VERSION,
                    "embeddingProfileId": "fixture-embedding:3",
                    "completedAt": "2026-07-15T12:00:00Z",
                    "reused": false,
                })
                .to_string(),
                stderr: String::new(),
                log_summary: vec![format!("Indexed {parent_item_key}")],
            });
        }
        let key = command_arg_value(command, "--attachment-key").unwrap();
        match key {
            "DIRECT" => {
                assert_eq!(command.label, "Zotero conversion worker");
                assert!(command.args.iter().any(|arg| arg == "--force-text"));
            }
            "SCAN" => {
                assert_eq!(command.label, "Zotero conversion worker");
                assert!(command.args.iter().any(|arg| arg == "--force-ocr"));
            }
            "MINERU" => {
                assert_eq!(command.label, ZOTERO_CONVERSION_COMMAND_LABEL);
                assert!(command.args.iter().any(|arg| arg == "--force-mineru"));
            }
            other => panic!("unexpected batch key {other}"),
        }
        fs::create_dir_all(&command.output_dir).unwrap();
        fs::write(
            command.output_dir.join(format!("{key}.md")),
            format!("---\nparent_item_key: \"{key}PARENT\"\n---\n\n# {key}\n"),
        )
        .unwrap();
        fs::write(
            command.output_dir.join(format!("{key}.json")),
            format!("{{\"key\":\"{key}\"}}\n"),
        )
        .unwrap();
        Ok(RunnerCommandResult {
            stdout: format!("Uploaded {key}.md to Zotero attachment {key}MD"),
            stderr: String::new(),
            log_summary: vec![format!("Batch fixture completed {key}")],
        })
    }
}

struct RetryCollectionExecutor {
    fail_once: std::sync::Mutex<bool>,
}

impl RunnerCommandExecutor for RetryCollectionExecutor {
    fn execute(&self, command: &RunnerCommand) -> Result<RunnerCommandResult, String> {
        if command.label == ITEM_INDEX_PROFILE_COMMAND_LABEL {
            return Ok(fixture_item_index_profile_result());
        }
        if command.label == ITEM_INDEX_COMMAND_LABEL {
            let parent_item_key = command_arg_value(command, "--parent-item-key").unwrap();
            let markdown_sha256 = command_arg_value(command, "--sha256").unwrap();
            return Ok(RunnerCommandResult {
                stdout: serde_json::json!({
                    "parentItemKey": parent_item_key,
                    "sourceSha256": markdown_sha256,
                    "chunkCount": 1,
                    "indexContractVersion": ITEM_INDEX_CONTRACT_VERSION,
                    "chunkContractVersion": CHUNK_CONTRACT_VERSION,
                    "embeddingProfileId": "fixture-embedding:3",
                    "completedAt": "2026-07-15T12:00:00Z",
                    "reused": false,
                })
                .to_string(),
                stderr: String::new(),
                log_summary: vec![format!("Indexed {parent_item_key}")],
            });
        }
        let key = command_arg_value(command, "--attachment-key").unwrap();
        if key == "FAIL" {
            let mut fail_once = self.fail_once.lock().unwrap();
            if *fail_once {
                *fail_once = false;
                return Err("item diagnosis: first attempt failed".into());
            }
        }
        fs::create_dir_all(&command.output_dir).unwrap();
        fs::write(
            command.output_dir.join(format!("{key}.md")),
            format!("---\nparent_item_key: \"{key}PARENT\"\n---\n\n# {key}\n"),
        )
        .unwrap();
        Ok(RunnerCommandResult {
            stdout: format!("Uploaded {key}.md to Zotero attachment {key}MD"),
            stderr: String::new(),
            log_summary: vec![format!("Retry fixture completed {key}")],
        })
    }
}

struct PanicExecutor;

impl RunnerCommandExecutor for PanicExecutor {
    fn execute(&self, _command: &RunnerCommand) -> Result<RunnerCommandResult, String> {
        panic!("fake Zotero discovery should not execute a command")
    }
}

fn temp_root(name: &str) -> PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("book-pipeline-{name}-{suffix}"))
}

fn fake_source(behavior: Option<&str>) -> BookPipelineSource {
    BookPipelineSource {
        kind: "fake".into(),
        title: Some("Fake source".into()),
        path: None,
        selector: None,
        runner_behavior: behavior.map(str::to_string),
        adapter_command: None,
        fake_zotero_items: None,
        route_overrides: BTreeMap::new(),
    }
}

/// A valid intent, so that a queue refusal can only have come from the mode.
fn fake_translation_intent() -> BookPipelineTranslationIntent {
    BookPipelineTranslationIntent {
        translation_mode: TRANSLATION_MODE_FAST.into(),
        profile_id: "fake-provider-profile".into(),
        config_id: "fake-provider-config".into(),
        skill_ids: Vec::new(),
        second_pass_enabled: false,
        text_cleanup: false,
        digest_mode: false,
        output_formats: default_output_formats(),
    }
}

fn local_pdf_source(input: &Path) -> BookPipelineSource {
    BookPipelineSource {
        kind: "local_pdf_folder".into(),
        title: Some("PDF folder".into()),
        path: Some(display_path(input)),
        selector: None,
        runner_behavior: None,
        adapter_command: None,
        fake_zotero_items: None,
        route_overrides: BTreeMap::new(),
    }
}

fn fake_wrapper_root(root: &Path) -> PathBuf {
    let wrapper_root = root.join("packages").join("ocr");
    let wrapper_script = wrapper_root
        .join("scripts")
        .join("pdf_to_html_paddleocr.py");
    fs::create_dir_all(wrapper_script.parent().unwrap()).unwrap();
    fs::write(&wrapper_script, "print('fixture')\n").unwrap();
    wrapper_root
}

fn fake_zotero_worker_root(root: &Path) -> PathBuf {
    let worker_root = root.join("packages").join("ocr");
    let worker_script = worker_root.join("scripts").join("zotero_llm_worker.py");
    fs::create_dir_all(worker_script.parent().unwrap()).unwrap();
    fs::write(&worker_script, "print('fixture')\n").unwrap();
    worker_root
}

fn fake_full_worker_root(root: &Path) -> PathBuf {
    let worker_root = fake_zotero_worker_root(root);
    fs::write(worker_root.join("mineru.py"), "print('mineru fixture')\n").unwrap();
    worker_root
}

fn has_arg_pair(args: &[String], key: &str, value: &str) -> bool {
    args.windows(2)
        .any(|pair| pair[0] == key && pair[1] == value)
}

fn has_env_pair(env: &[(String, String)], key: &str, value: &str) -> bool {
    env.iter()
        .any(|(env_key, env_value)| env_key == key && env_value == value)
}

fn fake_direct_zotero_source() -> BookPipelineSource {
    BookPipelineSource {
        kind: "zotero_attachment".into(),
        title: Some("Direct Text".into()),
        path: None,
        selector: Some("DIRECT".into()),
        runner_behavior: None,
        adapter_command: None,
        fake_zotero_items: Some(vec![FakeZoteroItem {
            key: "DIRECT".into(),
            title: "Direct Text".into(),
            attachment_path: Some("zotero://attachment/DIRECT".into()),
            has_text_layer: true,
            dirty_text_layer: false,
            scanned: false,
            already_converted: false,
            prefer_mineru: false,
        }]),
        route_overrides: BTreeMap::new(),
    }
}

fn markdown_source(path: &Path) -> BookPipelineSource {
    BookPipelineSource {
        kind: "markdown_source".into(),
        title: Some("Markdown Source".into()),
        path: Some(display_path(path)),
        selector: None,
        runner_behavior: None,
        adapter_command: None,
        fake_zotero_items: None,
        route_overrides: BTreeMap::new(),
    }
}

fn fake_mineru_zotero_source() -> BookPipelineSource {
    BookPipelineSource {
        kind: "zotero_attachment".into(),
        title: Some("MinerU Candidate".into()),
        path: None,
        selector: Some("MINERU".into()),
        runner_behavior: None,
        adapter_command: None,
        fake_zotero_items: Some(vec![FakeZoteroItem {
            key: "MINERU".into(),
            title: "MinerU Candidate".into(),
            attachment_path: Some("zotero://attachment/MINERU".into()),
            has_text_layer: false,
            dirty_text_layer: false,
            scanned: true,
            already_converted: false,
            prefer_mineru: true,
        }]),
        route_overrides: BTreeMap::new(),
    }
}

fn fake_collection_source() -> BookPipelineSource {
    BookPipelineSource {
        kind: "zotero_collection".into(),
        title: Some("Mixed collection".into()),
        path: None,
        selector: Some("COLLECTION".into()),
        runner_behavior: None,
        adapter_command: None,
        fake_zotero_items: Some(vec![
            FakeZoteroItem {
                key: "DIRECT".into(),
                title: "Direct Text".into(),
                attachment_path: Some("zotero://attachment/DIRECT".into()),
                has_text_layer: true,
                dirty_text_layer: false,
                scanned: false,
                already_converted: false,
                prefer_mineru: false,
            },
            FakeZoteroItem {
                key: "SCAN".into(),
                title: "Scanned PDF".into(),
                attachment_path: Some("zotero://attachment/SCAN".into()),
                has_text_layer: false,
                dirty_text_layer: false,
                scanned: true,
                already_converted: false,
                prefer_mineru: false,
            },
            FakeZoteroItem {
                key: "MINERU".into(),
                title: "MinerU Candidate".into(),
                attachment_path: Some("zotero://attachment/MINERU".into()),
                has_text_layer: false,
                dirty_text_layer: false,
                scanned: true,
                already_converted: false,
                prefer_mineru: true,
            },
            FakeZoteroItem {
                key: "DIRTY".into(),
                title: "Dirty Text Layer".into(),
                attachment_path: Some("zotero://attachment/DIRTY".into()),
                has_text_layer: true,
                dirty_text_layer: true,
                scanned: false,
                already_converted: false,
                prefer_mineru: false,
            },
            FakeZoteroItem {
                key: "DONE".into(),
                title: "Already Done".into(),
                attachment_path: Some("zotero://attachment/DONE".into()),
                has_text_layer: true,
                dirty_text_layer: false,
                scanned: false,
                already_converted: true,
                prefer_mineru: false,
            },
        ]),
        route_overrides: BTreeMap::new(),
    }
}

struct MemoryStateStore {
    state: Mutex<BookPipelineState>,
    output_root: PathBuf,
    save_count: Mutex<u32>,
    reject_save: bool,
}

impl MemoryStateStore {
    fn new(root: &Path) -> Self {
        Self {
            state: Mutex::new(BookPipelineState::default()),
            output_root: root.join("memory-output"),
            save_count: Mutex::new(0),
            reject_save: false,
        }
    }

    fn rejecting(root: &Path) -> Self {
        Self {
            reject_save: true,
            ..Self::new(root)
        }
    }
}

impl BookPipelineStateStore for MemoryStateStore {
    fn load(&self) -> Result<BookPipelineState, String> {
        Ok(self.state.lock().unwrap().clone())
    }

    fn save(&self, state: &BookPipelineState) -> Result<(), String> {
        if self.reject_save {
            return Err("fixture atomic save rejected".into());
        }
        let mut next = state.clone();
        next.revision = next.revision.saturating_add(1);
        *self.state.lock().unwrap() = next;
        *self.save_count.lock().unwrap() += 1;
        Ok(())
    }

    fn job_output_dir(&self, job_id: &str) -> PathBuf {
        self.output_root.join(job_id)
    }

    fn execution_owner(&self) -> Result<&str, String> {
        Ok("memory-state-owner")
    }
}

struct CollectionSnapshotExecutor {
    payload: String,
    calls: Mutex<u32>,
}

impl CollectionSnapshotExecutor {
    fn new(version: u64, include_new_member: bool) -> Self {
        let mut members = vec![
            serde_json::json!({
                "parentItemKey": "PARENT1",
                "parentItemType": "book",
                "parentItemVersion": 7,
                "parentDateModified": "2026-07-15 10:00:00",
                "title": "Eligible PDF",
                "attachmentKey": "PDFOK",
                "attachmentVersion": 21,
                "attachmentDateModified": "2026-07-15 11:00:00",
                "contentType": "application/pdf",
                "linkMode": 0,
                "storagePath": "storage:ok.pdf",
                "attachmentPath": "/private/zotero/PDFOK/ok.pdf",
                "pathExists": true,
                "fileSize": 1234,
                "fileMtimeNs": 111,
                "eligibility": "eligible_pdf",
                "reason": null,
            }),
            serde_json::json!({
                "parentItemKey": "PARENT2",
                "parentItemType": "book",
                "parentItemVersion": 8,
                "parentDateModified": "2026-07-15 10:01:00",
                "title": "Missing PDF",
                "attachmentKey": "PDFMISSING",
                "attachmentVersion": 22,
                "attachmentDateModified": "2026-07-15 11:01:00",
                "contentType": "application/pdf",
                "linkMode": 0,
                "storagePath": "storage:missing.pdf",
                "attachmentPath": "/private/zotero/PDFMISSING/missing.pdf",
                "pathExists": false,
                "fileSize": null,
                "fileMtimeNs": null,
                "eligibility": "missing_file",
                "reason": "PDF attachment file is missing."
            }),
            serde_json::json!({
                "parentItemKey": "PARENT3",
                "parentItemType": "book",
                "parentItemVersion": 9,
                "parentDateModified": "2026-07-15 10:02:00",
                "title": "Unsupported attachment",
                "attachmentKey": "TEXT1",
                "attachmentVersion": 23,
                "attachmentDateModified": "2026-07-15 11:02:00",
                "contentType": "text/plain",
                "linkMode": 0,
                "storagePath": "storage:notes.txt",
                "attachmentPath": "/private/zotero/TEXT1/notes.txt",
                "pathExists": true,
                "fileSize": 10,
                "fileMtimeNs": 222,
                "eligibility": "unsupported_content_type",
                "reason": "Unsupported attachment content type: text/plain."
            }),
            serde_json::json!({
                "parentItemKey": "PARENT4",
                "parentItemType": "book",
                "parentItemVersion": 10,
                "parentDateModified": "2026-07-15 10:03:00",
                "title": "No attachment",
                "attachmentKey": null,
                "attachmentVersion": null,
                "attachmentDateModified": null,
                "contentType": null,
                "linkMode": null,
                "storagePath": null,
                "attachmentPath": null,
                "pathExists": false,
                "fileSize": null,
                "fileMtimeNs": null,
                "eligibility": "no_attachment",
                "reason": "Collection member has no file attachment."
            }),
        ];
        if include_new_member {
            members.push(serde_json::json!({
                "parentItemKey": "PARENT5",
                "parentItemType": "journalArticle",
                "parentItemVersion": 11,
                "parentDateModified": "2026-07-15 10:04:00",
                "title": "New PDF",
                "attachmentKey": "PDFNEW",
                "attachmentVersion": 24,
                "attachmentDateModified": "2026-07-15 11:03:00",
                "contentType": "application/pdf",
                "linkMode": 0,
                "storagePath": "storage:new.pdf",
                "attachmentPath": "/private/zotero/PDFNEW/new.pdf",
                "pathExists": true,
                "fileSize": 5678,
                "fileMtimeNs": 333,
                "eligibility": "eligible_pdf",
                "reason": null,
            }));
        }
        for member in &mut members {
            member["collectionKey"] = serde_json::json!("COLL1");
        }
        Self {
            payload: serde_json::json!({
                "schemaVersion": "zotero-collection-snapshot-v1",
                "collection": {
                    "key": "COLL1",
                    "name": "Direct collection",
                    "version": version,
                },
                "members": members,
            })
            .to_string(),
            calls: Mutex::new(0),
        }
    }

    fn without_eligible_pdf() -> Self {
        let mut executor = Self::new(11, false);
        let mut payload: serde_json::Value = serde_json::from_str(&executor.payload).unwrap();
        payload["members"]
            .as_array_mut()
            .unwrap()
            .retain(|member| member["eligibility"] != "eligible_pdf");
        executor.payload = payload.to_string();
        executor
    }

    fn with_first_member_value(mut self, field: &str, value: serde_json::Value) -> Self {
        let mut payload: serde_json::Value = serde_json::from_str(&self.payload).unwrap();
        payload["members"][0][field] = value;
        self.payload = payload.to_string();
        self
    }

    fn mixed_routes(root: &Path) -> Self {
        let specs = [
            ("DIRECT", "PARENT1", "Direct PDF", "direct.pdf"),
            ("SCAN", "PARENT2", "Scanned PDF", "scan.pdf"),
            ("MINERU", "PARENT3", "MinerU PDF", "mineru.pdf"),
            ("DONE", "PARENT4", "Already completed PDF", "done.pdf"),
            ("FAIL", "PARENT5", "Failing PDF", "fail.pdf"),
            (
                "NOPADDLE",
                "PARENT6",
                "Paddle credential blocked PDF",
                "no-paddle.pdf",
            ),
        ];
        let mut members = specs
            .iter()
            .enumerate()
            .map(|(index, (attachment_key, parent_key, title, filename))| {
                let path = root.join("zotero").join(attachment_key).join(filename);
                fs::create_dir_all(path.parent().unwrap()).unwrap();
                fs::write(
                    &path,
                    format!("%PDF mixed collection fixture {attachment_key}\n"),
                )
                .unwrap();
                let metadata = fs::metadata(&path).unwrap();
                serde_json::json!({
                    "collectionKey": "COLL1",
                    "parentItemKey": parent_key,
                    "parentItemType": "book",
                    "parentItemVersion": 20 + index,
                    "parentDateModified": "2026-07-15 10:00:00",
                    "title": title,
                    "attachmentKey": attachment_key,
                    "attachmentVersion": 40 + index,
                    "attachmentDateModified": "2026-07-15 11:00:00",
                    "contentType": "application/pdf",
                    "linkMode": 0,
                    "storagePath": format!("storage:{filename}"),
                    "attachmentPath": display_path(&path),
                    "pathExists": true,
                    "fileSize": metadata.len(),
                    "fileMtimeNs": file_mtime_ns(&metadata).unwrap(),
                    "eligibility": "eligible_pdf",
                    "reason": null,
                })
            })
            .collect::<Vec<_>>();
        members.push(serde_json::json!({
            "collectionKey": "COLL1",
            "parentItemKey": "PARENT7",
            "parentItemType": "book",
            "parentItemVersion": 26,
            "parentDateModified": "2026-07-15 10:00:00",
            "title": "Missing PDF",
            "attachmentKey": "MISSING",
            "attachmentVersion": 46,
            "attachmentDateModified": "2026-07-15 11:00:00",
            "contentType": "application/pdf",
            "linkMode": 0,
            "storagePath": "storage:missing.pdf",
            "attachmentPath": display_path(&root.join("zotero/MISSING/missing.pdf")),
            "pathExists": false,
            "fileSize": null,
            "fileMtimeNs": null,
            "eligibility": "missing_file",
            "reason": "PDF attachment file is missing.",
        }));
        Self {
            payload: serde_json::json!({
                "schemaVersion": ZOTERO_COLLECTION_SNAPSHOT_SCHEMA,
                "collection": {
                    "key": "COLL1",
                    "name": "Mixed durable collection",
                    "version": 31,
                },
                "members": members,
            })
            .to_string(),
            calls: Mutex::new(0),
        }
    }

    fn recovery_routes(root: &Path) -> Self {
        let mut executor = Self::mixed_routes(root);
        let mut payload: serde_json::Value = serde_json::from_str(&executor.payload).unwrap();
        payload["members"].as_array_mut().unwrap().retain(|member| {
            member["attachmentKey"]
                .as_str()
                .is_some_and(|key| matches!(key, "DIRECT" | "SCAN" | "MINERU" | "DONE"))
        });
        executor.payload = payload.to_string();
        executor
    }
}

impl RunnerCommandExecutor for CollectionSnapshotExecutor {
    fn execute(&self, command: &RunnerCommand) -> Result<RunnerCommandResult, String> {
        assert_eq!(command.label, "Zotero collection snapshot");
        assert!(has_arg_pair(&command.args, "collection-snapshot", "COLL1"));
        *self.calls.lock().unwrap() += 1;
        Ok(RunnerCommandResult {
            stdout: self.payload.clone(),
            stderr: String::new(),
            log_summary: vec!["fixture private payload must not be persisted".into()],
        })
    }
}

fn collection_snapshot_executor_for_pdf(root: &Path) -> (CollectionSnapshotExecutor, PathBuf) {
    let pdf = root.join("zotero").join("PDFOK").join("ok.pdf");
    fs::create_dir_all(pdf.parent().unwrap()).unwrap();
    fs::write(&pdf, b"%PDF durable collection fixture\n").unwrap();
    let metadata = fs::metadata(&pdf).unwrap();
    let mtime_ns = metadata
        .modified()
        .unwrap()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64;
    let executor = CollectionSnapshotExecutor::new(11, false)
        .with_first_member_value("attachmentPath", serde_json::json!(display_path(&pdf)))
        .with_first_member_value("fileSize", serde_json::json!(metadata.len()))
        .with_first_member_value("fileMtimeNs", serde_json::json!(mtime_ns));
    (executor, pdf)
}

struct DurableCollectionChildExecutor {
    labels: Mutex<Vec<String>>,
    fail_index: bool,
}

impl DurableCollectionChildExecutor {
    fn new() -> Self {
        Self {
            labels: Mutex::new(Vec::new()),
            fail_index: false,
        }
    }

    fn failing_index() -> Self {
        Self {
            labels: Mutex::new(Vec::new()),
            fail_index: true,
        }
    }

    fn labels(&self) -> Vec<String> {
        self.labels.lock().unwrap().clone()
    }
}

impl RunnerCommandExecutor for DurableCollectionChildExecutor {
    fn execute(&self, command: &RunnerCommand) -> Result<RunnerCommandResult, String> {
        self.labels.lock().unwrap().push(command.label.clone());
        match command.label.as_str() {
            "Zotero discovery dry-run" => {
                assert!(has_arg_pair(&command.args, "--attachment-key", "PDFOK"));
                Ok(RunnerCommandResult {
                    stdout: "12:00:00 INFO PLAN PDFOK route=pdf-text pages=8 selected=8 parent_type=book sampled_chars=2000 title=Eligible PDF".into(),
                    stderr: String::new(),
                    log_summary: vec!["Single attachment route selected".into()],
                })
            }
            ZOTERO_CONVERSION_COMMAND_LABEL => {
                assert!(has_arg_pair(&command.args, "--attachment-key", "PDFOK"));
                assert!(command.args.iter().any(|arg| arg == "--force-text"));
                fs::create_dir_all(&command.output_dir).unwrap();
                fs::write(
                    command.output_dir.join("PDFOK.md"),
                    "---\nparent_item_key: \"PARENT1\"\n---\n\n# Extracted\n",
                )
                .unwrap();
                Ok(RunnerCommandResult {
                    stdout: "Uploaded PDFOK.md to Zotero attachment MARKDOWN1".into(),
                    stderr: String::new(),
                    log_summary: vec!["Single attachment extraction completed".into()],
                })
            }
            ITEM_INDEX_PROFILE_COMMAND_LABEL => Ok(fixture_item_index_profile_result()),
            ITEM_INDEX_COMMAND_LABEL => {
                if self.fail_index {
                    return Err("Fixture item index unavailable".into());
                }
                assert_eq!(
                    command_arg_value(command, "--parent-item-key"),
                    Some("PARENT1")
                );
                let markdown_sha256 = command_arg_value(command, "--sha256").unwrap();
                Ok(RunnerCommandResult {
                    stdout: serde_json::json!({
                        "parentItemKey": "PARENT1",
                        "sourceSha256": markdown_sha256,
                        "chunkCount": 1,
                        "indexContractVersion": ITEM_INDEX_CONTRACT_VERSION,
                        "chunkContractVersion": CHUNK_CONTRACT_VERSION,
                        "embeddingProfileId": "fixture-embedding:3",
                        "completedAt": "2026-07-15T12:00:00Z",
                        "reused": false,
                    })
                    .to_string(),
                    stderr: String::new(),
                    log_summary: vec!["Single attachment index completed".into()],
                })
            }
            other => panic!("unexpected durable collection child command {other}"),
        }
    }
}

struct MixedDurableCollectionExecutor {
    calls: Mutex<Vec<String>>,
    completed_markdown: PathBuf,
    completed_source_sha256: String,
    fail_index_once_parent: Mutex<Option<String>>,
    indexed_markdown_sha256: Mutex<BTreeMap<String, String>>,
}

impl MixedDurableCollectionExecutor {
    fn new(root: &Path) -> Self {
        let completed_markdown = root.join("already-completed").join("DONE.md");
        fs::create_dir_all(completed_markdown.parent().unwrap()).unwrap();
        fs::write(
            &completed_markdown,
            "---\nparent_item_key: \"PARENT4\"\n---\n\n# Reused\n",
        )
        .unwrap();
        Self {
            calls: Mutex::new(Vec::new()),
            completed_markdown,
            completed_source_sha256: sha256_file(
                &root.join("zotero").join("DONE").join("done.pdf"),
            )
            .unwrap(),
            fail_index_once_parent: Mutex::new(None),
            indexed_markdown_sha256: Mutex::new(BTreeMap::new()),
        }
    }

    fn failing_index_once(root: &Path, parent_item_key: &str) -> Self {
        let executor = Self::new(root);
        *executor.fail_index_once_parent.lock().unwrap() = Some(parent_item_key.into());
        executor
    }

    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }

    fn indexed_sha256(&self, parent_item_key: &str) -> Option<String> {
        self.indexed_markdown_sha256
            .lock()
            .unwrap()
            .get(parent_item_key)
            .cloned()
    }
}

impl RunnerCommandExecutor for MixedDurableCollectionExecutor {
    fn execute(&self, command: &RunnerCommand) -> Result<RunnerCommandResult, String> {
        match command.label.as_str() {
            "Zotero discovery dry-run" => {
                let key = command_arg_value(command, "--attachment-key")
                    .expect("durable routing must target one frozen attachment");
                assert!(command.args.iter().any(|arg| arg == "--pipeline-route"));
                self.calls.lock().unwrap().push(format!("route:{key}"));
                let stdout = match key {
                    "DIRECT" => "12:00:00 INFO PLAN DIRECT route=pdf-text pages=8 selected=8 parent_type=book sampled_chars=2000 title=Direct PDF".into(),
                    "SCAN" => "12:00:01 INFO PLAN SCAN route=paddle-ocr pages=20 selected=20 parent_type=book sampled_chars=0 title=Scanned PDF".into(),
                    "MINERU" => "12:00:02 INFO PLAN MINERU route=mineru pages=12 selected=12 parent_type=book sampled_chars=0 title=MinerU PDF".into(),
                    "DONE" => format!(
                        "12:00:03 INFO SKIP completed DONE Already completed PDF\n12:00:03 INFO BOOK_PIPELINE_ATTACHMENT_EVIDENCE {}",
                        serde_json::json!({
                            "schemaVersion": "zotero-worker-attachment-evidence-v1",
                            "extractionContractVersion": ZOTERO_WORKER_EXTRACTION_CONTRACT_VERSION,
                            "status": "already_completed",
                            "route": "pdf-text",
                            "pdfAttachmentKey": "DONE",
                            "parentItemKey": "PARENT4",
                            "sourceSha256": self.completed_source_sha256.clone(),
                            "markdownPath": display_path(&self.completed_markdown),
                            "markdownSha256": sha256_file(&self.completed_markdown).unwrap(),
                            "markdownAttachmentKey": "MARKDONE",
                        })
                    ),
                    "FAIL" => "12:00:04 INFO PLAN FAIL route=pdf-text pages=8 selected=8 parent_type=book sampled_chars=1800 title=Failing PDF".into(),
                    "NOPADDLE" => "12:00:05 INFO PLAN NOPADDLE route=missing-paddleocr-token pages=10 selected=10 parent_type=book sampled_chars=0 title=Paddle credential blocked PDF".into(),
                    other => panic!("unexpected mixed route attachment {other}"),
                };
                Ok(RunnerCommandResult {
                    stdout,
                    stderr: String::new(),
                    log_summary: vec!["Per-attachment route fixture completed".into()],
                })
            }
            ZOTERO_CONVERSION_COMMAND_LABEL => {
                let key = command_arg_value(command, "--attachment-key")
                    .expect("mixed extraction must target one frozen attachment");
                self.calls.lock().unwrap().push(format!("extract:{key}"));
                assert!(command.args.iter().any(|arg| arg == "--preserve-source"));
                match key {
                    "DIRECT" => assert!(command.args.iter().any(|arg| arg == "--force-text")),
                    "SCAN" => assert!(command.args.iter().any(|arg| arg == "--force-ocr")),
                    "MINERU" => {
                        assert!(command.args.iter().any(|arg| arg == "--force-mineru"))
                    }
                    "FAIL" => return Err("fixture attachment extraction failed".into()),
                    other => panic!("unexpected mixed extraction attachment {other}"),
                }
                let parent = match key {
                    "DIRECT" => "PARENT1",
                    "SCAN" => "PARENT2",
                    "MINERU" => "PARENT3",
                    _ => unreachable!(),
                };
                fs::create_dir_all(&command.output_dir).unwrap();
                fs::write(
                    command.output_dir.join(format!("{key}.md")),
                    format!("---\nparent_item_key: \"{parent}\"\n---\n\n# {key}\n"),
                )
                .unwrap();
                Ok(RunnerCommandResult {
                    stdout: format!("Uploaded {key}.md to Zotero attachment MARK{key}"),
                    stderr: String::new(),
                    log_summary: vec!["Single attachment extraction completed".into()],
                })
            }
            ITEM_INDEX_PROFILE_COMMAND_LABEL => Ok(fixture_item_index_profile_result()),
            ITEM_INDEX_COMMAND_LABEL => {
                let parent = command_arg_value(command, "--parent-item-key").unwrap();
                self.calls.lock().unwrap().push(format!("index:{parent}"));
                let markdown_sha256 = command_arg_value(command, "--sha256").unwrap();
                let mut fail_once = self.fail_index_once_parent.lock().unwrap();
                if fail_once.as_deref() == Some(parent) {
                    fail_once.take();
                    return Err("fixture item index interrupted once".into());
                }
                drop(fail_once);
                self.indexed_markdown_sha256
                    .lock()
                    .unwrap()
                    .insert(parent.to_string(), markdown_sha256.to_string());
                Ok(RunnerCommandResult {
                    stdout: serde_json::json!({
                        "parentItemKey": parent,
                        "sourceSha256": markdown_sha256,
                        "chunkCount": 1,
                        "indexContractVersion": ITEM_INDEX_CONTRACT_VERSION,
                        "chunkContractVersion": CHUNK_CONTRACT_VERSION,
                        "embeddingProfileId": "fixture-embedding:3",
                        "completedAt": "2026-07-15T12:00:00Z",
                        "reused": false,
                    })
                    .to_string(),
                    stderr: String::new(),
                    log_summary: vec!["Single attachment index completed".into()],
                })
            }
            other => panic!("unexpected mixed collection command {other}"),
        }
    }
}

struct PanicPipelineRunner;

impl PipelineRunner for PanicPipelineRunner {
    fn run(&self, _job: &BookPipelineJob, _output_dir: &Path) -> Result<RunnerOutput, String> {
        panic!("durable collection execution must not invoke the batch runner")
    }

    fn route_attachment(
        &self,
        _job: &BookPipelineJob,
        child: &BookPipelineChildJob,
        _output_dir: &Path,
    ) -> Result<AttachmentRouteOutput, String> {
        Ok(AttachmentRouteOutput {
            route: BookPipelineRouteItem {
                id: child.source.selector.clone().unwrap(),
                title: child.source.title.clone().unwrap(),
                source_kind: "zotero_attachment".into(),
                source_ref: child.source.path.clone().unwrap(),
                route_kind: "direct_text".into(),
                can_run: true,
                blocked_reason: None,
                summary: "Fixture direct attachment route".into(),
                route_override: None,
            },
            log_summary: vec!["Fixture attachment route selected".into()],
            reused_artifact: None,
        })
    }
}

fn real_collection_source() -> BookPipelineSource {
    BookPipelineSource {
        kind: "zotero_collection".into(),
        title: Some("Selected collection".into()),
        path: None,
        selector: Some("COLL1".into()),
        runner_behavior: None,
        adapter_command: None,
        fake_zotero_items: None,
        route_overrides: BTreeMap::new(),
    }
}

fn fast_translation_intent() -> BookPipelineTranslationIntent {
    BookPipelineTranslationIntent {
        translation_mode: TRANSLATION_MODE_FAST.into(),
        profile_id: "fixture-profile".into(),
        config_id: "fixture-config".into(),
        skill_ids: Vec::new(),
        second_pass_enabled: false,
        text_cleanup: false,
        digest_mode: false,
        output_formats: default_output_formats(),
    }
}

fn cleanup_fixture_job(
    root: &Path,
    store: &BookPipelineStore,
    zotero_key: Option<&str>,
) -> BookPipelineJob {
    let output_dir = root.join("cleanup-output");
    fs::create_dir_all(&output_dir).unwrap();
    let markdown = output_dir.join("book.md");
    fs::write(&markdown, "# Clean Markdown\n").unwrap();
    let source_pdf = root.join("source.pdf");
    fs::write(&source_pdf, "%PDF fixture").unwrap();
    let mut source = fake_direct_zotero_source();
    source.path = Some(display_path(&source_pdf));
    let job = queue_job(
        store,
        source,
        "conversion_only".into(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();
    let mut state = store.load().unwrap();
    let stored = state
        .jobs
        .iter_mut()
        .find(|stored| stored.id == job.id)
        .unwrap();
    stored.status = STATUS_COMPLETED.into();
    stored.current_step = "Completed".into();
    stored.output_dir = Some(display_path(&output_dir));
    stored.artifacts = vec![
        BookPipelineArtifact {
            kind: "output_dir".into(),
            path: display_path(&output_dir),
            sha256: None,
            zotero_key: None,
            producer_stage: None,
            ..BookPipelineArtifact::default()
        },
        BookPipelineArtifact {
            kind: "markdown".into(),
            path: display_path(&markdown),
            sha256: Some(sha256_file(&markdown).unwrap()),
            zotero_key: zotero_key.map(str::to_string),
            producer_stage: None,
            ..BookPipelineArtifact::default()
        },
    ];
    let job = stored.clone();
    store.save(&state).unwrap();
    job
}

#[test]
fn delete_job_removes_a_queued_job_and_persists() {
    let root = temp_root("delete-queued-job");
    let store = BookPipelineStore::for_test(&root);
    let job = queue_job(
        &store,
        fake_source(None),
        "conversion_only".into(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();

    let state = delete_job(&store, &job.id, None, true).unwrap();

    assert!(state.jobs.is_empty());
    assert!(store.load().unwrap().jobs.is_empty());
    let _ = fs::remove_dir_all(root);
}

// Deleting the job for the book the user pointed at took the whole batch —
// and the child set cannot simply shrink: `validate_state` requires
// `child_job_ids` to equal the children exactly and
// `validate_state_transitions` rejects any membership change. So the book is
// marked, not removed, and everything that walks the children skips it.
#[test]
fn dropping_one_book_of_a_batch_keeps_the_rest_and_the_frozen_membership() {
    let root = temp_root("drop-one-of-a-batch");
    let store = BookPipelineStore::for_test(&root);
    let job = queue_job(
        &store,
        fake_collection_source(),
        "conversion_only".into(),
        BookPipelinePreviewConfig {
            has_paddleocr_credentials: true,
            has_mineru_credentials: true,
            route_overrides: BTreeMap::new(),
        },
    )
    .unwrap();
    assert!(job.children.len() > 1, "the fixture queues a batch");
    let dropped = job.children[0].id.clone();
    let survivors = job.children.len() - 1;
    let membership_before = job.membership.clone();

    let state = delete_job(&store, &job.id, Some(&dropped), true).unwrap();

    let stored = state.jobs.iter().find(|item| item.id == job.id).unwrap();
    assert_eq!(
        stored.children.len(),
        survivors + 1,
        "the child stays; only the shelf loses it"
    );
    assert!(stored
        .children
        .iter()
        .find(|child| child.id == dropped)
        .unwrap()
        .removed_at
        .is_some());
    assert_eq!(
        stored.membership, membership_before,
        "the frozen membership must be untouched, or the save would be rejected"
    );
    assert_eq!(
        stored.summary.total, survivors as u32,
        "a dropped book is not counted any more"
    );
    assert!(live_children(stored).all(|child| child.id != dropped));

    // A dropped book must never be picked up for work again.
    assert!(locate_child_index(stored, Some(&dropped)).is_err());
    assert_ne!(locate_child_index(stored, None).unwrap(), 0);

    // Dropping the last one that is left removes the job: an empty batch is
    // a shelf row nobody could dismiss.
    for child in stored.children.clone() {
        if child.removed_at.is_none() {
            delete_job(&store, &job.id, Some(&child.id), true).unwrap();
        }
    }
    assert!(store.load().unwrap().jobs.is_empty());

    let _ = fs::remove_dir_all(root);
}

// A single-book job has nothing to narrow to: the child is the job.
#[test]
fn dropping_the_only_book_removes_the_job() {
    let root = temp_root("drop-only-book");
    let store = BookPipelineStore::for_test(&root);
    let job = queue_job(
        &store,
        fake_source(None),
        "conversion_only".into(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();
    let child_id = job.children[0].id.clone();

    let state = delete_job(&store, &job.id, Some(&child_id), true).unwrap();

    assert!(state.jobs.is_empty());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn delete_job_requires_explicit_approval_and_a_known_job() {
    let root = temp_root("delete-job-guards");
    let store = BookPipelineStore::for_test(&root);
    let job = queue_job(
        &store,
        fake_source(None),
        "conversion_only".into(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();

    let refused = delete_job(&store, &job.id, None, false).unwrap_err();
    assert!(refused.contains("Explicit approval"), "got: {refused}");
    assert_eq!(store.load().unwrap().jobs.len(), 1);

    let missing = delete_job(&store, "job-nonexistent", None, true).unwrap_err();
    assert!(missing.contains("not found"), "got: {missing}");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_job_with_a_running_stage_counts_as_actively_running() {
    let root = temp_root("delete-running-guard");
    let store = BookPipelineStore::for_test(&root);
    let job = queue_job(
        &store,
        fake_source(None),
        "conversion_only".into(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();

    assert!(!job_is_actively_running(&job));
    let mut running = job;
    start_stage(&mut running.children[0], "extract", "test-owner");
    assert!(job_is_actively_running(&running));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn book_ocr_conversion_root_prefers_monorepo_ocr_package() {
    let root = book_ocr_conversion_root();

    assert!(
        root.ends_with(Path::new("packages").join("ocr")),
        "expected monorepo OCR package root, got {}",
        display_path(&root)
    );
}

#[test]
fn legacy_jobs_migrate_to_versioned_parent_child_stage_state() {
    let root = temp_root("legacy-state-migration");
    let store = BookPipelineStore::for_test(&root);
    fs::create_dir_all(store.state_path.parent().unwrap()).unwrap();
    // `translationStrategy` below is a retired field that still exists in
    // state files written before it was removed; keeping it in the fixture
    // proves the loader ignores it instead of failing the migration.
    let legacy_job = |id: &str, status: &str| {
        serde_json::json!({
            "id": id,
            "mode": "convert_then_translate",
            "source": {
                "kind": "zotero_attachment",
                "title": "Fabricated source",
                "path": "zotero://attachment/FAKEPDF",
                "selector": "FAKEPDF",
                "runnerBehavior": null,
                "translationStrategy": "reflection",
                "adapterCommand": null,
                "fakeZoteroItems": null
            },
            "route": [{
                "id": "FAKEPDF",
                "title": "Fabricated source",
                "sourceKind": "zotero_attachment",
                "sourceRef": "zotero://attachment/FAKEPDF",
                "routeKind": "direct_text",
                "canRun": true,
                "blockedReason": null,
                "summary": "Fabricated route"
            }],
            "status": status,
            "currentStep": "Legacy state",
            "lastError": "preserved diagnosis",
            "logSummary": ["preserved log"],
            "artifacts": [{
                "kind": "markdown",
                "path": "/tmp/fabricated.md",
                "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "zoteroKey": "FAKEMD"
            }],
            "collectionItems": [{
                "id": "FAKEPDF",
                "title": "Fabricated source",
                "routeKind": "direct_text",
                "status": "completed",
                "lastError": null,
                "artifacts": [],
                "attempts": 2
            }],
            "outputDir": "/tmp/fabricated-output",
            "attempts": 2,
            "createdAt": "2026-07-10T09:00:00+08:00",
            "updatedAt": "2026-07-10T09:05:00+08:00"
        })
    };
    let legacy = serde_json::json!({
        "jobs": [
            legacy_job("legacy-routed", STATUS_ROUTED),
            legacy_job("legacy-handoff", STATUS_HANDOFF_RUNNING),
            legacy_job("legacy-ready", STATUS_TRANSLATION_READY)
        ]
    });
    fs::write(
        &store.state_path,
        serde_json::to_string_pretty(&legacy).unwrap(),
    )
    .unwrap();

    let state = store.load().unwrap();

    assert_eq!(state.schema_version, STATE_SCHEMA_VERSION);
    assert_eq!(state.revision, 1);
    let routed = state
        .jobs
        .iter()
        .find(|job| job.id == "legacy-routed")
        .unwrap();
    assert_eq!(routed.schema_version, JOB_SCHEMA_VERSION);
    assert_eq!(routed.translation_mode, TRANSLATION_MODE_FAST);
    assert!(!routed.second_pass_enabled);
    assert!(!routed.text_cleanup);
    assert_eq!(routed.status, STATUS_READY);
    assert_eq!(routed.attempts, 2);
    assert_eq!(routed.last_error.as_deref(), Some("preserved diagnosis"));
    assert_eq!(routed.collection_items.len(), 1);
    assert_eq!(routed.artifacts.len(), 1);
    assert_eq!(routed.children.len(), 1);
    assert_eq!(routed.children[0].current_stage_id, "extract");
    assert_eq!(routed.children[0].status, STATUS_READY);

    let handoff = state
        .jobs
        .iter()
        .find(|job| job.id == "legacy-handoff")
        .unwrap();
    assert_eq!(handoff.current_stage_id, "handoff");
    assert_eq!(handoff.status, STATUS_RUNNING);
    assert_eq!(handoff.children[0].status, STATUS_RUNNING);
    assert_eq!(
        child_stage_status(handoff, "index"),
        STATUS_SKIPPED,
        "legacy downstream progress must not fabricate completed index evidence"
    );
    assert!(handoff.children[0]
        .stages
        .iter()
        .find(|stage| stage.stage_id == "handoff")
        .unwrap()
        .execution_owner
        .is_some());

    let translation_ready = state
        .jobs
        .iter()
        .find(|job| job.id == "legacy-ready")
        .unwrap();
    assert_eq!(translation_ready.current_stage_id, "split");
    assert_eq!(translation_ready.status, STATUS_READY);
    assert_eq!(translation_ready.children[0].status, STATUS_READY);
    assert_eq!(
        translation_ready.children[0]
            .stages
            .iter()
            .find(|stage| stage.stage_id == "handoff")
            .unwrap()
            .status,
        STATUS_COMPLETED
    );

    let persisted: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&store.state_path).unwrap()).unwrap();
    assert_eq!(persisted["schemaVersion"], STATE_SCHEMA_VERSION);
    assert_eq!(persisted["revision"], 1);
    assert!(persisted["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .all(|job| job["schemaVersion"] == JOB_SCHEMA_VERSION));

    let recovered_after_migration = store.load().unwrap();
    let handoff = recovered_after_migration
        .jobs
        .iter()
        .find(|job| job.id == "legacy-handoff")
        .unwrap();
    assert_eq!(handoff.current_stage_id, "handoff");
    assert_eq!(handoff.status, STATUS_FAILED);
    assert!(handoff.children[0]
        .stages
        .iter()
        .find(|stage| stage.stage_id == "handoff")
        .unwrap()
        .error
        .as_deref()
        .unwrap()
        .contains("interrupted"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn retired_conversion_only_mode_is_refused_at_the_queue() {
    let root = temp_root("retired-mode-enqueue");
    let store = BookPipelineStore::for_test(&root);

    let error = queue_job_with_translation_intent(
        &store,
        fake_source(None),
        MODE_CONVERSION_ONLY.into(),
        fake_translation_intent(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap_err();

    assert!(
        error.contains("retired") && error.contains(MODE_CONVERT_THEN_TRANSLATE),
        "the refusal must say the mode is retired and name the replacement, got {error}"
    );
    assert!(
        store.load().unwrap().jobs.is_empty(),
        "a refused enqueue must not leave a job behind"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn unknown_mode_is_refused_instead_of_inheriting_a_truncated_pipeline() {
    let root = temp_root("unknown-mode-enqueue");
    let store = BookPipelineStore::for_test(&root);

    let error = queue_job_with_translation_intent(
        &store,
        fake_source(None),
        "convert_then_translat".into(),
        fake_translation_intent(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap_err();

    assert!(
        error.contains("convert_then_translat") && error.contains(MODE_TRANSLATE_ONLY),
        "the refusal must quote the rejected mode and list the valid ones, got {error}"
    );
    assert!(store.load().unwrap().jobs.is_empty());

    // The retirement's point: a mode that is merely unrecognised must no longer
    // fall through to the conversion-only shape and stop short of translation.
    assert!(
        should_handoff_after_run("convert_then_translat"),
        "only the named retired mode may skip the translation handoff"
    );
    assert!(
        ordered_child_stage_ids("convert_then_translat", false).contains(&"handoff"),
        "an unrecognised mode must not be given the truncated stage list"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn stored_conversion_only_jobs_still_open_and_keep_their_three_stage_shape() {
    let root = temp_root("retired-mode-stored-state");
    let store = BookPipelineStore::for_test(&root);
    fs::create_dir_all(store.state_path.parent().unwrap()).unwrap();
    // A checkpoint written before the mode was retired. Retiring it closed the
    // queue, not the library: this file still has to open.
    let legacy = serde_json::json!({
        "jobs": [{
            "id": "stored-conversion-only",
            "mode": MODE_CONVERSION_ONLY,
            "source": {
                "kind": "zotero_attachment",
                "title": "Fabricated source",
                "path": "zotero://attachment/FAKEPDF",
                "selector": "FAKEPDF",
                "runnerBehavior": null,
                "adapterCommand": null,
                "fakeZoteroItems": null
            },
            "route": [{
                "id": "FAKEPDF",
                "title": "Fabricated source",
                "sourceKind": "zotero_attachment",
                "sourceRef": "zotero://attachment/FAKEPDF",
                "routeKind": "direct_text",
                "canRun": true,
                "blockedReason": null,
                "summary": "Fabricated route"
            }],
            "status": STATUS_ROUTED,
            "currentStep": "Legacy conversion-only state",
            "lastError": null,
            "logSummary": ["preserved log"],
            "artifacts": [],
            "collectionItems": [],
            "outputDir": "/tmp/fabricated-output",
            "attempts": 1,
            "createdAt": "2026-07-10T09:00:00+08:00",
            "updatedAt": "2026-07-10T09:05:00+08:00"
        }]
    });
    fs::write(
        &store.state_path,
        serde_json::to_string_pretty(&legacy).unwrap(),
    )
    .unwrap();

    let state = store.load().unwrap();

    let stored = &state.jobs[0];
    assert_eq!(
        stored.mode, MODE_CONVERSION_ONLY,
        "the stored mode must be preserved, not rewritten to a live one"
    );
    assert_eq!(
        stored.children[0]
            .stages
            .iter()
            .map(|stage| stage.stage_id.as_str())
            .collect::<Vec<_>>(),
        vec!["route", "extract", "index"],
        "migration must not graft translation stages onto a finished conversion-only job"
    );
    assert!(!should_handoff_after_run(&stored.mode));

    // Reload proves the migrated file it just wrote is itself still loadable.
    let reloaded = store.load().unwrap();
    assert_eq!(reloaded.jobs[0].mode, MODE_CONVERSION_ONLY);
    assert_eq!(reloaded.jobs[0].children[0].stages.len(), 3);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn queued_translation_modes_and_binding_identity_survive_persistence() {
    let root = temp_root("translation-mode-persistence");
    let store = BookPipelineStore::for_test(&root);
    let fast = queue_job_with_translation_intent(
        &store,
        fake_source(None),
        MODE_CONVERT_THEN_TRANSLATE.into(),
        BookPipelineTranslationIntent {
            translation_mode: TRANSLATION_MODE_FAST.into(),
            profile_id: "fake-provider-profile".into(),
            config_id: "fake-provider-config".into(),
            skill_ids: Vec::new(),
            second_pass_enabled: true,
            text_cleanup: true,
            digest_mode: false,
            output_formats: default_output_formats(),
        },
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();
    let expert = queue_job_with_translation_intent(
        &store,
        fake_source(None),
        MODE_CONVERT_THEN_TRANSLATE.into(),
        BookPipelineTranslationIntent {
            translation_mode: TRANSLATION_MODE_EXPERT.into(),
            profile_id: "fake-agent-profile".into(),
            config_id: "fake-agent-config".into(),
            skill_ids: vec!["expert-translation-quality".into()],
            second_pass_enabled: false,
            text_cleanup: false,
            digest_mode: false,
            output_formats: default_output_formats(),
        },
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();

    let state = store.load().unwrap();
    let persisted_fast = state.jobs.iter().find(|job| job.id == fast.id).unwrap();
    assert_eq!(persisted_fast.translation_mode, TRANSLATION_MODE_FAST);
    assert_eq!(
        persisted_fast.translation_profile_id,
        "fake-provider-profile"
    );
    assert_eq!(persisted_fast.translation_config_id, "fake-provider-config");
    assert!(persisted_fast.translation_skill_ids.is_empty());
    assert!(persisted_fast.second_pass_enabled);
    assert!(persisted_fast.text_cleanup);
    let persisted_expert = state.jobs.iter().find(|job| job.id == expert.id).unwrap();
    assert_eq!(persisted_expert.translation_mode, TRANSLATION_MODE_EXPERT);
    assert_eq!(
        persisted_expert.translation_profile_id,
        "fake-agent-profile"
    );
    assert_eq!(persisted_expert.translation_config_id, "fake-agent-config");
    assert_eq!(
        persisted_expert.translation_skill_ids,
        vec!["expert-translation-quality"]
    );
    assert!(!persisted_expert.second_pass_enabled);
    assert!(!persisted_expert.text_cleanup);

    let persisted_json: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&store.state_path).unwrap()).unwrap();
    assert!(persisted_json["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .all(|job| matches!(job["translationMode"].as_str(), Some("fast" | "expert"))));
    assert!(persisted_json["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .any(|job| job["translationMode"] == "fast" && job["secondPassEnabled"] == true));
    assert!(persisted_json["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .any(|job| job["translationMode"] == "fast" && job["textCleanup"] == true));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn queued_digest_mode_is_book_level_and_survives_persistence() {
    let root = temp_root("digest-mode-persistence");
    let store = BookPipelineStore::for_test(&root);
    let fast_intent: BookPipelineTranslationIntent = serde_json::from_value(serde_json::json!({
        "translationMode": TRANSLATION_MODE_FAST,
        "profileId": "fake-provider-profile",
        "configId": "fake-provider-config",
        "skillIds": [],
        "secondPassEnabled": false,
        "digestMode": true,
    }))
    .unwrap();
    let expert_intent: BookPipelineTranslationIntent = serde_json::from_value(serde_json::json!({
        "translationMode": TRANSLATION_MODE_EXPERT,
        "profileId": "fake-agent-profile",
        "configId": "fake-agent-config",
        "skillIds": ["expert-translation-quality"],
        "secondPassEnabled": false,
        "digestMode": true,
    }))
    .unwrap();
    let fast = queue_job_with_translation_intent(
        &store,
        fake_source(None),
        MODE_CONVERT_THEN_TRANSLATE.into(),
        fast_intent,
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();
    let expert = queue_job_with_translation_intent(
        &store,
        fake_source(None),
        MODE_CONVERT_THEN_TRANSLATE.into(),
        expert_intent,
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();

    let persisted: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&store.state_path).unwrap()).unwrap();
    for job_id in [fast.id, expert.id] {
        let job = persisted["jobs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|job| job["id"] == job_id)
            .unwrap();
        assert_eq!(job["digestMode"], true);
        assert_eq!(
            job["outputFormats"],
            serde_json::json!(["md", "html", "epub"])
        );
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn queued_output_formats_are_ordered_deduplicated_and_persisted() {
    let root = temp_root("output-formats-persistence");
    let store = BookPipelineStore::for_test(&root);
    let job = queue_job_with_translation_intent(
        &store,
        fake_source(None),
        MODE_CONVERT_THEN_TRANSLATE.into(),
        BookPipelineTranslationIntent {
            translation_mode: TRANSLATION_MODE_FAST.into(),
            profile_id: "fake-provider-profile".into(),
            config_id: "fake-provider-config".into(),
            skill_ids: Vec::new(),
            second_pass_enabled: false,
            text_cleanup: false,
            digest_mode: false,
            output_formats: vec![
                "bilingual".into(),
                "epub".into(),
                "BILINGUAL".into(),
                "md".into(),
            ],
        },
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();

    assert_eq!(job.output_formats, vec!["bilingual", "epub", "md"]);
    let persisted: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&store.state_path).unwrap()).unwrap();
    let stored = persisted["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|candidate| candidate["id"] == job.id)
        .unwrap();
    assert_eq!(
        stored["outputFormats"],
        serde_json::json!(["bilingual", "epub", "md"])
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn expert_translation_intent_rejects_fast_only_second_pass() {
    let error = validate_translation_intent(&BookPipelineTranslationIntent {
        translation_mode: TRANSLATION_MODE_EXPERT.into(),
        profile_id: "fake-agent-profile".into(),
        config_id: "fake-agent-config".into(),
        skill_ids: vec!["expert-translation-quality".into()],
        second_pass_enabled: true,
        text_cleanup: false,
        digest_mode: false,
        output_formats: default_output_formats(),
    })
    .unwrap_err();

    assert!(error.contains("only available in fast translation mode"));
}

#[test]
fn expert_translation_intent_rejects_fast_only_text_cleanup() {
    let error = validate_translation_intent(&BookPipelineTranslationIntent {
        translation_mode: TRANSLATION_MODE_EXPERT.into(),
        profile_id: "fake-agent-profile".into(),
        config_id: "fake-agent-config".into(),
        skill_ids: vec!["expert-translation-quality".into()],
        second_pass_enabled: false,
        text_cleanup: true,
        digest_mode: false,
        output_formats: default_output_formats(),
    })
    .unwrap_err();

    assert!(error.contains("only available in fast translation mode"));
}

#[test]
fn legacy_collection_migration_preserves_route_union_and_parent_handoff_state() {
    let root = temp_root("legacy-collection-state-migration");
    let store = BookPipelineStore::for_test(&root);
    fs::create_dir_all(store.state_path.parent().unwrap()).unwrap();
    let legacy = serde_json::json!({
        "jobs": [{
            "id": "legacy-collection-handoff",
            "mode": MODE_CONVERT_THEN_TRANSLATE,
            "source": {
                "kind": "zotero_collection",
                "title": "Fabricated collection",
                "path": null,
                "selector": "FAKECOLL",
                "runnerBehavior": null,
                "translationStrategy": null,
                "adapterCommand": null,
                "fakeZoteroItems": null
            },
            "route": [{
                "id": "ROUTEONLY",
                "title": "Route-only attachment",
                "sourceKind": "zotero_attachment",
                "sourceRef": "zotero://attachment/ROUTEONLY",
                "routeKind": "direct_text",
                "canRun": true,
                "blockedReason": null,
                "summary": "Fabricated route"
            }],
            "status": STATUS_HANDOFF_RUNNING,
            "currentStep": "Legacy collection handoff",
            "lastError": null,
            "logSummary": [],
            "artifacts": [],
            "collectionItems": [{
                "id": "RESULTONLY",
                "title": "Result-only attachment",
                "routeKind": "direct_text",
                "status": STATUS_COMPLETED,
                "lastError": null,
                "artifacts": [],
                "attempts": 3
            }],
            "outputDir": null,
            "attempts": 3,
            "createdAt": "2026-07-10T09:00:00+08:00",
            "updatedAt": "2026-07-10T09:05:00+08:00"
        }]
    });
    fs::write(
        &store.state_path,
        serde_json::to_string_pretty(&legacy).unwrap(),
    )
    .unwrap();

    let state = store.load().unwrap();
    let job = &state.jobs[0];

    assert_eq!(job.status, STATUS_RUNNING);
    assert_eq!(job.children.len(), 2);
    assert_eq!(
        job.children
            .iter()
            .map(|child| child.source.selector.as_deref().unwrap())
            .collect::<std::collections::BTreeSet<_>>(),
        std::collections::BTreeSet::from(["RESULTONLY", "ROUTEONLY"])
    );
    let handoff_child = job
        .children
        .iter()
        .find(|child| child.source.selector.as_deref() == Some("RESULTONLY"))
        .unwrap();
    assert_eq!(handoff_child.current_stage_id, "handoff");
    assert_eq!(handoff_child.status, STATUS_RUNNING);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn collection_parent_derives_partial_status_from_durable_children() {
    let root = temp_root("collection-status-aggregation");
    let store = BookPipelineStore::for_test(&root);
    let job = queue_job(
        &store,
        fake_collection_source(),
        "conversion_only".into(),
        BookPipelinePreviewConfig {
            has_paddleocr_credentials: true,
            has_mineru_credentials: true,
            route_overrides: BTreeMap::new(),
        },
    )
    .unwrap();

    assert_eq!(job.kind, "collection");
    assert_eq!(job.children.len(), 5);
    assert_eq!(
        job.membership.as_ref().unwrap().child_job_ids.len(),
        job.children.len()
    );

    let mut state = store.load().unwrap();
    let stored = state
        .jobs
        .iter_mut()
        .find(|stored| stored.id == job.id)
        .unwrap();
    for child in &mut stored.children {
        if matches!(child.source.selector.as_deref(), Some("DIRECT" | "SCAN")) {
            start_stage(child, "extract", store.execution_owner().unwrap());
        }
    }
    store.save(&state).unwrap();

    let mut state = store.load().unwrap();
    let stored = state
        .jobs
        .iter_mut()
        .find(|stored| stored.id == job.id)
        .unwrap();
    for child in &mut stored.children {
        let status = match child.source.selector.as_deref().unwrap() {
            "DIRECT" => STATUS_COMPLETED,
            "SCAN" => STATUS_FAILED,
            "MINERU" | "DIRTY" => STATUS_BLOCKED,
            "DONE" => STATUS_SKIPPED,
            other => panic!("unexpected fixture child {other}"),
        };
        child.stages[0].status = if child.source.selector.as_deref() == Some("DIRTY") {
            STATUS_BLOCKED.into()
        } else if status == STATUS_SKIPPED {
            STATUS_SKIPPED.into()
        } else {
            STATUS_COMPLETED.into()
        };
        if child.source.selector.as_deref() == Some("DIRTY") {
            set_stage_status(child, "extract", STATUS_PENDING, None);
        } else {
            set_stage_status(child, "extract", status, None);
        }
        if status == STATUS_COMPLETED {
            set_stage_status(child, "index", STATUS_SKIPPED, None);
        }
    }
    store.save(&state).unwrap();

    let recovered = store.load().unwrap();
    let parent = recovered
        .jobs
        .iter()
        .find(|stored| stored.id == job.id)
        .unwrap();
    assert_eq!(parent.status, STATUS_PARTIAL);
    assert_eq!(parent.summary.total, 5);
    assert_eq!(parent.summary.completed, 1);
    assert_eq!(parent.summary.failed, 1);
    assert_eq!(parent.summary.blocked, 2);
    assert_eq!(parent.summary.skipped, 1);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn versioned_stage_units_and_approvals_survive_restart() {
    let root = temp_root("versioned-restart-recovery");
    let store = BookPipelineStore::for_test(&root);
    let job = queue_job(
        &store,
        fake_source(None),
        MODE_CONVERT_THEN_TRANSLATE.into(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();
    let mut state = store.load().unwrap();
    let stored = state
        .jobs
        .iter_mut()
        .find(|stored| stored.id == job.id)
        .unwrap();
    let child = stored.children.first_mut().unwrap();
    let translate = child
        .stages
        .iter_mut()
        .find(|stage| stage.stage_id == "translate")
        .unwrap();
    translate.unit_summary = Some(BookPipelineUnitSummary {
        total: 2,
        completed: 1,
        failed: 1,
        ..BookPipelineUnitSummary::default()
    });
    stored.approval_references = vec![BookPipelineApprovalReference {
        approval_id: "approval-fake-1".into(),
        gate_id: "translation_disclosure".into(),
        child_job_id: child.id.clone(),
        stage_id: "approve_translation".into(),
        decision: "approved".into(),
        bound_artifact_hashes: std::collections::BTreeMap::from([(
            "task-manifest".into(),
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
        )]),
        decided_at: now_label(),
    }];
    store.save(&state).unwrap();

    let restarted_store = BookPipelineStore::for_test(&root);
    let recovered = restarted_store.load().unwrap();
    let recovered_job = recovered
        .jobs
        .iter()
        .find(|stored| stored.id == job.id)
        .unwrap();
    let recovered_child = recovered_job.children.first().unwrap();
    assert_eq!(
        recovered_child
            .stages
            .iter()
            .map(|stage| stage.stage_id.as_str())
            .collect::<Vec<_>>(),
        vec![
            "route",
            "extract",
            "index",
            "handoff",
            "split",
            "prepare",
            "approve_translation",
            "translate",
            "expert_qa",
            "approve_promotion",
            "promote",
            "build_reading",
            "validate_reading",
            "build_digest"
        ]
    );
    assert_eq!(
        recovered_child
            .stages
            .iter()
            .find(|stage| stage.stage_id == "build_digest")
            .unwrap()
            .status,
        STATUS_SKIPPED
    );
    let unit_summary = recovered_child
        .stages
        .iter()
        .find(|stage| stage.stage_id == "translate")
        .unwrap()
        .unit_summary
        .as_ref()
        .unwrap();
    assert_eq!(unit_summary.total, 2);
    assert_eq!(unit_summary.completed, 1);
    assert_eq!(unit_summary.failed, 1);
    assert_eq!(recovered_job.approval_references.len(), 1);
    assert!(recovered_child
        .stages
        .iter()
        .all(|stage| !stage.contract_version.is_empty()));
    assert_eq!(
        recovered_job.approval_references[0]
            .bound_artifact_hashes
            .get("task-manifest")
            .map(String::as_str),
        Some("bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb")
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn foreign_running_stage_recovers_as_retryable_failure_after_restart() {
    let root = temp_root("interrupted-stage-recovery");
    let store = BookPipelineStore::for_test_with_owner(&root, "worker-before-restart");
    let job = queue_job(
        &store,
        fake_source(None),
        "conversion_only".into(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();
    store.execution_owner().unwrap();
    let mut state = store.load().unwrap();
    let extract = state
        .jobs
        .iter_mut()
        .find(|stored| stored.id == job.id)
        .unwrap()
        .children[0]
        .stages
        .iter_mut()
        .find(|stage| stage.stage_id == "extract")
        .unwrap();
    extract.status = STATUS_RUNNING.into();
    extract.execution_owner = Some("worker-before-restart".into());
    store.save(&state).unwrap();
    let running_revision = store.load().unwrap().revision;
    drop(store);

    let restarted = BookPipelineStore::for_test_with_owner(&root, "worker-after-restart")
        .load()
        .unwrap();
    let recovered = restarted
        .jobs
        .iter()
        .find(|stored| stored.id == job.id)
        .unwrap();
    let extract = recovered.children[0]
        .stages
        .iter()
        .find(|stage| stage.stage_id == "extract")
        .unwrap();

    assert_eq!(extract.status, STATUS_FAILED);
    assert!(extract.error.as_deref().unwrap().contains("interrupted"));
    assert_eq!(extract.execution_owner, None);
    assert_eq!(restarted.revision, running_revision + 1);
    let _ = fs::remove_dir_all(root);
}

#[cfg(any(unix, target_os = "windows"))]
#[test]
fn live_foreign_execution_owner_is_not_mistaken_for_restart() {
    let root = temp_root("live-foreign-stage-owner");
    #[cfg(unix)]
    let mut live_process = Command::new("sh")
        .args(["-c", "while :; do sleep 1; done"])
        .spawn()
        .unwrap();
    #[cfg(target_os = "windows")]
    let mut live_process = Command::new("powershell")
        .args(["-NoProfile", "-Command", "Start-Sleep -Seconds 30"])
        .spawn()
        .unwrap();
    let writer_owner = format!("process-{}-writer", live_process.id());
    let reader_owner = new_execution_owner();
    let writer = BookPipelineStore::for_test_with_owner(&root, &writer_owner);
    let job = queue_job(
        &writer,
        fake_source(None),
        "conversion_only".into(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();
    writer.execution_owner().unwrap();
    let mut state = writer.load().unwrap();
    let extract = state
        .jobs
        .iter_mut()
        .find(|stored| stored.id == job.id)
        .unwrap()
        .children[0]
        .stages
        .iter_mut()
        .find(|stage| stage.stage_id == "extract")
        .unwrap();
    extract.status = STATUS_RUNNING.into();
    extract.execution_owner = Some(writer_owner.clone());
    writer.save(&state).unwrap();
    let running_revision = writer.load().unwrap().revision;

    let observed = BookPipelineStore::for_test_with_owner(&root, &reader_owner)
        .load()
        .unwrap();
    let extract = observed
        .jobs
        .iter()
        .find(|stored| stored.id == job.id)
        .unwrap()
        .children[0]
        .stages
        .iter()
        .find(|stage| stage.stage_id == "extract")
        .unwrap();

    assert_eq!(extract.status, STATUS_RUNNING);
    assert_eq!(
        extract.execution_owner.as_deref(),
        Some(writer_owner.as_str())
    );
    assert_eq!(observed.revision, running_revision);
    live_process.kill().unwrap();
    live_process.wait().unwrap();
    let _ = fs::remove_dir_all(root);
}

#[cfg(any(unix, target_os = "windows"))]
#[test]
fn live_unrelated_pid_without_matching_lease_is_interrupted() {
    let root = temp_root("live-unrelated-pid-stage-owner");
    #[cfg(unix)]
    let mut live_process = Command::new("sh")
        .args(["-c", "while :; do sleep 1; done"])
        .spawn()
        .unwrap();
    #[cfg(target_os = "windows")]
    let mut live_process = Command::new("powershell")
        .args(["-NoProfile", "-Command", "Start-Sleep -Seconds 30"])
        .spawn()
        .unwrap();
    let store = BookPipelineStore::for_test(&root);
    let job = queue_job(
        &store,
        fake_source(None),
        "conversion_only".into(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();
    let mut state = store.load().unwrap();
    let extract = state
        .jobs
        .iter_mut()
        .find(|stored| stored.id == job.id)
        .unwrap()
        .children[0]
        .stages
        .iter_mut()
        .find(|stage| stage.stage_id == "extract")
        .unwrap();
    extract.status = STATUS_RUNNING.into();
    extract.execution_owner = Some(format!("process-{}-stale", live_process.id()));
    store.write_state_unlocked(&state).unwrap();

    let recovered = store.load().unwrap();
    let extract = recovered
        .jobs
        .iter()
        .find(|stored| stored.id == job.id)
        .unwrap()
        .children[0]
        .stages
        .iter()
        .find(|stage| stage.stage_id == "extract")
        .unwrap();

    assert_eq!(extract.status, STATUS_FAILED);
    live_process.kill().unwrap();
    live_process.wait().unwrap();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn reused_current_pid_does_not_keep_stale_owner_alive() {
    let root = temp_root("reused-current-pid-stage-owner");
    let stale_owner = format!("process-{}-stale-owner", std::process::id());
    let store = BookPipelineStore::for_test(&root);
    let job = queue_job(
        &store,
        fake_source(None),
        "conversion_only".into(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();
    let mut state = store.load().unwrap();
    let extract = state
        .jobs
        .iter_mut()
        .find(|stored| stored.id == job.id)
        .unwrap()
        .children[0]
        .stages
        .iter_mut()
        .find(|stage| stage.stage_id == "extract")
        .unwrap();
    extract.status = STATUS_RUNNING.into();
    extract.execution_owner = Some(stale_owner);
    store.write_state_unlocked(&state).unwrap();

    let recovered = store.load().unwrap();
    let extract = recovered
        .jobs
        .iter()
        .find(|stored| stored.id == job.id)
        .unwrap()
        .children[0]
        .stages
        .iter()
        .find(|stage| stage.stage_id == "extract")
        .unwrap();

    assert_eq!(extract.status, STATUS_FAILED);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn ownerless_running_stage_recovers_as_interrupted() {
    let root = temp_root("ownerless-stage-recovery");
    let store = BookPipelineStore::for_test(&root);
    let job = queue_job(
        &store,
        fake_source(None),
        "conversion_only".into(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();
    let mut state = store.load().unwrap();
    let extract = state
        .jobs
        .iter_mut()
        .find(|stored| stored.id == job.id)
        .unwrap()
        .children[0]
        .stages
        .iter_mut()
        .find(|stage| stage.stage_id == "extract")
        .unwrap();
    extract.status = STATUS_RUNNING.into();
    extract.execution_owner = None;
    store.write_state_unlocked(&state).unwrap();

    let recovered = store.load().unwrap();
    let extract = recovered
        .jobs
        .iter()
        .find(|stored| stored.id == job.id)
        .unwrap()
        .children[0]
        .stages
        .iter()
        .find(|stage| stage.stage_id == "extract")
        .unwrap();

    assert_eq!(extract.status, STATUS_FAILED);
    assert!(extract.error.as_deref().unwrap().contains("interrupted"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn frozen_collection_membership_rejects_child_drift() {
    let root = temp_root("frozen-collection-membership");
    let store = BookPipelineStore::for_test(&root);
    let job = queue_job(
        &store,
        fake_collection_source(),
        "conversion_only".into(),
        BookPipelinePreviewConfig {
            has_paddleocr_credentials: true,
            has_mineru_credentials: true,
            route_overrides: BTreeMap::new(),
        },
    )
    .unwrap();
    let mut state = store.load().unwrap();
    state
        .jobs
        .iter_mut()
        .find(|stored| stored.id == job.id)
        .unwrap()
        .children
        .pop();

    let error = store.save(&state).unwrap_err();

    assert!(error.contains("frozen membership"));
    assert_eq!(
        store
            .load()
            .unwrap()
            .jobs
            .iter()
            .find(|stored| stored.id == job.id)
            .unwrap()
            .children
            .len(),
        job.children.len()
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn real_collection_discovery_atomically_freezes_durable_attachment_children() {
    let root = temp_root("collection-snapshot-freeze");
    let store = MemoryStateStore::new(&root);
    let executor = CollectionSnapshotExecutor::new(11, false);

    let job = queue_job_with_translation_intent_and_executor(
        &store,
        &executor,
        real_collection_source(),
        "conversion_only".into(),
        fast_translation_intent(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();

    assert_eq!(*executor.calls.lock().unwrap(), 1);
    assert_eq!(*store.save_count.lock().unwrap(), 1);
    assert_eq!(job.kind, "collection");
    assert!(job.collection_items.is_empty());
    let membership = job.membership.as_ref().unwrap();
    assert_eq!(membership.collection_key, "COLL1");
    assert_eq!(membership.revision, 1);
    assert_eq!(membership.snapshot_sha256.len(), 64);
    assert_eq!(membership.child_job_ids.len(), 3);
    assert_eq!(job.children.len(), 3);
    assert!(job
        .children
        .iter()
        .all(|child| child.id.contains("-r1-") && child.parent_job_id == job.id));
    let eligible = job
        .children
        .iter()
        .find(|child| child.source.selector.as_deref() == Some("PDFOK"))
        .unwrap();
    assert_eq!(stage_ref(eligible, "route").unwrap().status, STATUS_READY);
    assert_eq!(
        stage_ref(eligible, "extract").unwrap().status,
        STATUS_PENDING
    );
    let identity = eligible.source_identity.as_ref().unwrap();
    assert_eq!(identity.collection_key, "COLL1");
    assert_eq!(identity.parent_item_key, "PARENT1");
    assert_eq!(identity.pdf_attachment_key, "PDFOK");
    assert_eq!(identity.content_type, "application/pdf");
    assert_eq!(identity.file_size, Some(1234));
    assert_eq!(identity.file_mtime_ns, Some(111));
    let missing = job
        .children
        .iter()
        .find(|child| child.source.selector.as_deref() == Some("PDFMISSING"))
        .unwrap();
    assert_eq!(stage_ref(missing, "route").unwrap().status, STATUS_BLOCKED);
    assert!(stage_ref(missing, "route")
        .unwrap()
        .error
        .as_deref()
        .is_some_and(|error| error.contains("missing")));
    assert!(job.artifacts.iter().any(|artifact| {
        artifact.kind == "collection_manifest"
            && artifact.sha256.is_some()
            && artifact.producer.stage_id == "discover"
    }));
    let persisted = store.load().unwrap();
    assert_eq!(persisted.jobs.len(), 1);
    assert_eq!(persisted.jobs[0].membership, job.membership);
    assert_eq!(persisted.jobs[0].children.len(), 3);
    assert!(!serde_json::to_string(&persisted)
        .unwrap()
        .contains("fixture private payload"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn identical_collection_snapshot_is_idempotent_and_changed_snapshot_revises() {
    let root = temp_root("collection-snapshot-idempotency");
    let store = MemoryStateStore::new(&root);
    let first_executor = CollectionSnapshotExecutor::new(11, false);
    let first = queue_job_with_translation_intent_and_executor(
        &store,
        &first_executor,
        real_collection_source(),
        "conversion_only".into(),
        fast_translation_intent(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();
    let same_executor = CollectionSnapshotExecutor::new(11, false);

    let same = queue_job_with_translation_intent_and_executor(
        &store,
        &same_executor,
        real_collection_source(),
        "conversion_only".into(),
        fast_translation_intent(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();

    assert_eq!(same.id, first.id);
    assert_eq!(*store.save_count.lock().unwrap(), 1);
    assert_eq!(store.load().unwrap().jobs.len(), 1);

    let changed_executor = CollectionSnapshotExecutor::new(12, true);
    let changed = queue_job_with_translation_intent_and_executor(
        &store,
        &changed_executor,
        real_collection_source(),
        "conversion_only".into(),
        fast_translation_intent(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();

    assert_ne!(changed.id, first.id);
    assert_eq!(changed.membership.as_ref().unwrap().revision, 2);
    assert_eq!(changed.children.len(), 4);
    assert_eq!(store.load().unwrap().jobs.len(), 2);
    assert_eq!(*store.save_count.lock().unwrap(), 2);
    let old = store
        .load()
        .unwrap()
        .jobs
        .into_iter()
        .find(|job| job.id == first.id)
        .unwrap();
    assert_eq!(old.membership.as_ref().unwrap().revision, 1);
    assert_eq!(old.children.len(), 3);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn rejected_state_save_publishes_neither_membership_nor_children() {
    let root = temp_root("collection-snapshot-atomic-rejection");
    let store = MemoryStateStore::rejecting(&root);
    let executor = CollectionSnapshotExecutor::new(11, false);

    let error = queue_job_with_translation_intent_and_executor(
        &store,
        &executor,
        real_collection_source(),
        "conversion_only".into(),
        fast_translation_intent(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap_err();

    assert!(error.contains("atomic save rejected"));
    assert!(store.load().unwrap().jobs.is_empty());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn collection_snapshot_without_eligible_pdf_is_durably_explainable() {
    let root = temp_root("collection-snapshot-no-eligible-pdf");
    let store = MemoryStateStore::new(&root);
    let executor = CollectionSnapshotExecutor::without_eligible_pdf();

    let job = queue_job_with_translation_intent_and_executor(
        &store,
        &executor,
        real_collection_source(),
        "conversion_only".into(),
        fast_translation_intent(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();

    assert_eq!(job.status, STATUS_BLOCKED);
    assert_eq!(job.current_stage_id, "discover");
    assert_eq!(job.last_error.as_deref(), Some("no_eligible_pdf"));
    assert_eq!(job.children.len(), 2);
    assert!(job
        .children
        .iter()
        .all(|child| stage_ref(child, "route").unwrap().status == STATUS_BLOCKED));
    assert_eq!(store.load().unwrap().jobs.len(), 1);
    assert_eq!(*store.save_count.lock().unwrap(), 1);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn collection_snapshot_rejects_inconsistent_eligible_file_evidence() {
    for (case, executor) in [
        (
            "missing-path",
            CollectionSnapshotExecutor::new(11, false)
                .with_first_member_value("pathExists", serde_json::json!(false)),
        ),
        (
            "non-pdf-content",
            CollectionSnapshotExecutor::new(11, false)
                .with_first_member_value("contentType", serde_json::json!("text/plain")),
        ),
        (
            "wrong-link-mode",
            CollectionSnapshotExecutor::new(11, false)
                .with_first_member_value("linkMode", serde_json::json!(2)),
        ),
        (
            "mismatched-resolved-path",
            CollectionSnapshotExecutor::new(11, false).with_first_member_value(
                "attachmentPath",
                serde_json::json!("/private/zotero/OTHER/secret.pdf"),
            ),
        ),
    ] {
        let root = temp_root(&format!("collection-snapshot-invalid-{case}"));
        let store = MemoryStateStore::new(&root);

        let error = queue_job_with_translation_intent_and_executor(
            &store,
            &executor,
            real_collection_source(),
            "conversion_only".into(),
            fast_translation_intent(),
            BookPipelinePreviewConfig::default(),
        )
        .unwrap_err();

        assert!(error.contains("inconsistent attachment evidence"));
        assert!(store.load().unwrap().jobs.is_empty());
        let _ = fs::remove_dir_all(root);
    }
}

#[test]
fn public_collection_discovery_and_preview_use_readonly_snapshot_contract() {
    let discovery_executor = CollectionSnapshotExecutor::new(11, false);

    let discovery =
        discover_zotero_sources(&discovery_executor, &real_collection_source(), 20).unwrap();

    assert_eq!(*discovery_executor.calls.lock().unwrap(), 1);
    assert_eq!(discovery.sources.len(), 1);
    assert_eq!(discovery.sources[0].kind, "zotero_collection");
    assert_eq!(discovery.sources[0].selector.as_deref(), Some("COLL1"));
    assert_eq!(
        discovery.sources[0].title.as_deref(),
        Some("Direct collection")
    );
    assert!(discovery.sources[0].fake_zotero_items.is_none());
    assert!(discovery
        .log_summary
        .iter()
        .any(|line| line.contains("members=4 attachments=3 eligible=1")));

    let preview_executor = CollectionSnapshotExecutor::new(11, false);
    let preview = preview_book_pipeline_route_with_executor(
        &preview_executor,
        &real_collection_source(),
        "conversion_only",
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();

    assert_eq!(*preview_executor.calls.lock().unwrap(), 1);
    assert_eq!(preview.len(), 3);
    assert!(preview.iter().any(|route| {
        route.id == "PDFOK" && route.route_kind == "pending_route" && route.can_run
    }));
    assert!(preview.iter().any(|route| {
        route.id == "PDFMISSING" && route.route_kind == "missing_file" && !route.can_run
    }));
}

#[test]
fn durable_collection_run_claims_route_without_invoking_batch_runner() {
    let root = temp_root("collection-durable-route-claim");
    let store = MemoryStateStore::new(&root);
    let (executor, _) = collection_snapshot_executor_for_pdf(&root);
    let queued = queue_job_with_translation_intent_and_executor(
        &store,
        &executor,
        real_collection_source(),
        "conversion_only".into(),
        fast_translation_intent(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();

    let routed = run_job(&store, &PanicPipelineRunner, &queued.id).unwrap();

    assert_eq!(routed.id, queued.id);
    assert!(routed
        .children
        .iter()
        .any(|child| stage_ref(child, "route").unwrap().status == STATUS_COMPLETED));
    assert!(routed
        .children
        .iter()
        .filter(|child| stage_ref(child, "route").unwrap().status == STATUS_COMPLETED)
        .all(|child| stage_ref(child, "extract").unwrap().status == STATUS_READY));
    assert_eq!(*store.save_count.lock().unwrap(), 3);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn durable_collection_child_runs_route_extract_index_and_handoff_in_order() {
    let root = temp_root("durable-collection-child-chain");
    let worker_root = fake_zotero_worker_root(&root);
    let repo_root = root.join("repo");
    let store = BookPipelineStore::for_test(&root);
    let (snapshot_executor, _) = collection_snapshot_executor_for_pdf(&root);
    let queued = queue_job_with_translation_intent_and_executor(
        &store,
        &snapshot_executor,
        real_collection_source(),
        MODE_CONVERT_THEN_TRANSLATE.into(),
        fast_translation_intent(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();
    let child_id = queued
        .children
        .iter()
        .find(|child| child.source.selector.as_deref() == Some("PDFOK"))
        .unwrap()
        .id
        .clone();
    let runner = CommandPipelineRunner::with_book_ocr_conversion_root(
        DurableCollectionChildExecutor::new(),
        worker_root,
    );

    let routed = run_job_with_handoff(
        &store,
        &runner,
        &FakeTranslationHandoffRunner,
        &queued.id,
        Some(&repo_root),
    )
    .unwrap();

    assert_eq!(routed.children.len(), queued.children.len());
    assert!(routed.collection_items.is_empty());
    let child = routed
        .children
        .iter()
        .find(|child| child.id == child_id)
        .unwrap();
    assert_eq!(stage_ref(child, "route").unwrap().status, STATUS_COMPLETED);
    assert_eq!(stage_ref(child, "extract").unwrap().status, STATUS_READY);
    assert_eq!(stage_ref(child, "index").unwrap().status, STATUS_PENDING);
    assert_eq!(runner.executor.labels(), vec!["Zotero discovery dry-run"]);
    assert_eq!(
        stage_ref(
            store
                .load()
                .unwrap()
                .jobs
                .iter()
                .find(|job| job.id == queued.id)
                .unwrap()
                .children
                .iter()
                .find(|child| child.id == child_id)
                .unwrap(),
            "extract",
        )
        .unwrap()
        .status,
        STATUS_READY
    );

    let extracted = run_job_with_handoff(
        &store,
        &runner,
        &FakeTranslationHandoffRunner,
        &queued.id,
        Some(&repo_root),
    )
    .unwrap();
    let child = extracted
        .children
        .iter()
        .find(|child| child.id == child_id)
        .unwrap();
    assert_eq!(
        stage_ref(child, "extract").unwrap().status,
        STATUS_COMPLETED
    );
    assert_eq!(stage_ref(child, "index").unwrap().status, STATUS_READY);
    assert_eq!(stage_ref(child, "handoff").unwrap().status, STATUS_PENDING);
    let markdown = child
        .artifacts
        .iter()
        .find(|artifact| artifact.kind == "markdown")
        .unwrap();
    assert_eq!(
        markdown.source_refs.collection_key.as_deref(),
        Some("COLL1")
    );
    assert_eq!(
        markdown.source_refs.parent_item_key.as_deref(),
        Some("PARENT1")
    );
    assert_eq!(
        markdown.source_refs.pdf_attachment_key.as_deref(),
        Some("PDFOK")
    );
    assert_eq!(
        markdown.source_refs.markdown_attachment_key.as_deref(),
        Some("MARKDOWN1")
    );
    assert_eq!(
        runner.executor.labels(),
        vec!["Zotero discovery dry-run", ZOTERO_CONVERSION_COMMAND_LABEL]
    );
    assert_eq!(
        stage_ref(
            store
                .load()
                .unwrap()
                .jobs
                .iter()
                .find(|job| job.id == queued.id)
                .unwrap()
                .children
                .iter()
                .find(|child| child.id == child_id)
                .unwrap(),
            "index",
        )
        .unwrap()
        .status,
        STATUS_READY
    );

    let indexed = run_job_with_handoff(
        &store,
        &runner,
        &FakeTranslationHandoffRunner,
        &queued.id,
        Some(&repo_root),
    )
    .unwrap();
    let child = indexed
        .children
        .iter()
        .find(|child| child.id == child_id)
        .unwrap();
    assert_eq!(stage_ref(child, "index").unwrap().status, STATUS_COMPLETED);
    assert_eq!(stage_ref(child, "handoff").unwrap().status, STATUS_READY);
    assert!(stage_ref(child, "index").unwrap().index_evidence.is_some());
    assert_eq!(
        runner.executor.labels(),
        vec![
            "Zotero discovery dry-run",
            ZOTERO_CONVERSION_COMMAND_LABEL,
            ITEM_INDEX_PROFILE_COMMAND_LABEL,
            ITEM_INDEX_COMMAND_LABEL,
        ]
    );
    assert_eq!(
        stage_ref(
            store
                .load()
                .unwrap()
                .jobs
                .iter()
                .find(|job| job.id == queued.id)
                .unwrap()
                .children
                .iter()
                .find(|child| child.id == child_id)
                .unwrap(),
            "handoff",
        )
        .unwrap()
        .status,
        STATUS_READY
    );

    let handed_off = run_job_with_handoff(
        &store,
        &runner,
        &FakeTranslationHandoffRunner,
        &queued.id,
        Some(&repo_root),
    )
    .unwrap();
    let child = handed_off
        .children
        .iter()
        .find(|child| child.id == child_id)
        .unwrap();
    assert_eq!(
        stage_ref(child, "handoff").unwrap().status,
        STATUS_COMPLETED
    );
    assert_eq!(stage_ref(child, "split").unwrap().status, STATUS_READY);
    assert!(child
        .artifacts
        .iter()
        .any(|artifact| artifact.kind == "translation_source"));
    assert_eq!(handed_off.children.len(), queued.children.len());
    assert!(handed_off.collection_items.is_empty());
    assert_eq!(store.load().unwrap().jobs.len(), 1);
    assert_eq!(
        runner.executor.labels(),
        vec![
            "Zotero discovery dry-run",
            ZOTERO_CONVERSION_COMMAND_LABEL,
            ITEM_INDEX_PROFILE_COMMAND_LABEL,
            ITEM_INDEX_COMMAND_LABEL,
        ]
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn durable_collection_index_failure_preserves_extract_and_blocks_handoff() {
    let root = temp_root("durable-collection-child-index-failure");
    let worker_root = fake_zotero_worker_root(&root);
    let repo_root = root.join("repo");
    let store = BookPipelineStore::for_test(&root);
    let (snapshot_executor, _) = collection_snapshot_executor_for_pdf(&root);
    let queued = queue_job_with_translation_intent_and_executor(
        &store,
        &snapshot_executor,
        real_collection_source(),
        MODE_CONVERT_THEN_TRANSLATE.into(),
        fast_translation_intent(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();
    let child_id = queued
        .children
        .iter()
        .find(|child| child.source.selector.as_deref() == Some("PDFOK"))
        .unwrap()
        .id
        .clone();
    let runner = CommandPipelineRunner::with_book_ocr_conversion_root(
        DurableCollectionChildExecutor::failing_index(),
        worker_root,
    );

    run_job_with_handoff(
        &store,
        &runner,
        &FakeTranslationHandoffRunner,
        &queued.id,
        Some(&repo_root),
    )
    .unwrap();
    run_job_with_handoff(
        &store,
        &runner,
        &FakeTranslationHandoffRunner,
        &queued.id,
        Some(&repo_root),
    )
    .unwrap();
    let failed = run_job_with_handoff(
        &store,
        &runner,
        &FakeTranslationHandoffRunner,
        &queued.id,
        Some(&repo_root),
    )
    .unwrap();

    let child = failed
        .children
        .iter()
        .find(|child| child.id == child_id)
        .unwrap();
    assert_eq!(
        stage_ref(child, "extract").unwrap().status,
        STATUS_COMPLETED
    );
    assert_eq!(stage_ref(child, "index").unwrap().status, STATUS_FAILED);
    assert_eq!(stage_ref(child, "handoff").unwrap().status, STATUS_PENDING);
    assert!(child
        .artifacts
        .iter()
        .any(|artifact| artifact.kind == "markdown"));
    assert!(!child
        .artifacts
        .iter()
        .any(|artifact| artifact.kind == "translation_source"));
    assert_eq!(failed.children.len(), queued.children.len());
    assert!(failed.collection_items.is_empty());
    let labels_before_repeat = runner.executor.labels();

    let repeated = retry_job_with_handoff(
        &store,
        &runner,
        &FakeTranslationHandoffRunner,
        &queued.id,
        Some(&repo_root),
    )
    .unwrap();

    let labels_after_retry = runner.executor.labels();
    assert_eq!(labels_after_retry.len(), labels_before_repeat.len() + 1);
    assert_eq!(labels_after_retry.last().unwrap(), ITEM_INDEX_COMMAND_LABEL);
    assert_eq!(repeated.children.len(), queued.children.len());
    let child = repeated
        .children
        .iter()
        .find(|child| child.id == child_id)
        .unwrap();
    assert_eq!(stage_ref(child, "extract").unwrap().attempt, 1);
    assert_eq!(stage_ref(child, "index").unwrap().attempt, 2);
    assert_eq!(stage_ref(child, "handoff").unwrap().status, STATUS_PENDING);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn durable_collection_missing_extract_artifact_fails_index_without_reextracting() {
    let root = temp_root("durable-collection-missing-extract-artifact");
    let store = BookPipelineStore::for_test(&root);
    let (snapshot_executor, _) = collection_snapshot_executor_for_pdf(&root);
    let queued = queue_job_with_translation_intent_and_executor(
        &store,
        &snapshot_executor,
        real_collection_source(),
        MODE_CONVERT_THEN_TRANSLATE.into(),
        fast_translation_intent(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();
    let runner = CommandPipelineRunner::with_book_ocr_conversion_root(
        DurableCollectionChildExecutor::new(),
        fake_zotero_worker_root(&root),
    );

    run_job_with_handoff(
        &store,
        &runner,
        &FakeTranslationHandoffRunner,
        &queued.id,
        Some(&root.join("repo")),
    )
    .unwrap();
    let extracted = run_job_with_handoff(
        &store,
        &runner,
        &FakeTranslationHandoffRunner,
        &queued.id,
        Some(&root.join("repo")),
    )
    .unwrap();
    let child = extracted
        .children
        .iter()
        .find(|child| child.source.selector.as_deref() == Some("PDFOK"))
        .unwrap();
    let markdown_path = child
        .artifacts
        .iter()
        .find(|artifact| artifact.kind == "markdown")
        .unwrap()
        .path
        .clone();
    fs::remove_file(markdown_path).unwrap();
    let labels_before_index = runner.executor.labels();

    let failed = run_job_with_handoff(
        &store,
        &runner,
        &FakeTranslationHandoffRunner,
        &queued.id,
        Some(&root.join("repo")),
    )
    .unwrap();

    assert_eq!(runner.executor.labels(), labels_before_index);
    let child = failed
        .children
        .iter()
        .find(|child| child.source.selector.as_deref() == Some("PDFOK"))
        .unwrap();
    assert_eq!(
        stage_ref(child, "extract").unwrap().status,
        STATUS_COMPLETED
    );
    assert_eq!(stage_ref(child, "extract").unwrap().attempt, 1);
    assert_eq!(stage_ref(child, "index").unwrap().status, STATUS_FAILED);
    assert_eq!(stage_ref(child, "index").unwrap().attempt, 1);
    assert_eq!(stage_ref(child, "handoff").unwrap().status, STATUS_PENDING);
    assert!(child
        .last_error
        .as_deref()
        .is_some_and(|error| !error.trim().is_empty()));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn durable_collection_blocks_when_frozen_pdf_changes_before_route() {
    let root = temp_root("durable-collection-source-drift");
    let store = BookPipelineStore::for_test(&root);
    let (snapshot_executor, pdf) = collection_snapshot_executor_for_pdf(&root);
    let queued = queue_job_with_translation_intent_and_executor(
        &store,
        &snapshot_executor,
        real_collection_source(),
        MODE_CONVERT_THEN_TRANSLATE.into(),
        fast_translation_intent(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();
    fs::write(
        &pdf,
        b"%PDF replaced after discovery with different bytes\n",
    )
    .unwrap();
    let runner = CommandPipelineRunner::with_book_ocr_conversion_root(
        DurableCollectionChildExecutor::new(),
        fake_zotero_worker_root(&root),
    );

    let blocked = run_job_with_handoff(
        &store,
        &runner,
        &FakeTranslationHandoffRunner,
        &queued.id,
        Some(&root.join("repo")),
    )
    .unwrap();

    let child = blocked
        .children
        .iter()
        .find(|child| child.source.selector.as_deref() == Some("PDFOK"))
        .unwrap();
    assert_eq!(stage_ref(child, "route").unwrap().status, STATUS_BLOCKED);
    assert_eq!(stage_ref(child, "extract").unwrap().status, STATUS_PENDING);
    assert!(child
        .last_error
        .as_deref()
        .is_some_and(|error| error.contains("frozen source")));
    assert!(runner.executor.labels().is_empty());
    assert_eq!(blocked.children.len(), queued.children.len());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn durable_collection_blocks_handoff_when_indexed_markdown_changes() {
    let root = temp_root("durable-collection-markdown-drift");
    let worker_root = fake_zotero_worker_root(&root);
    let repo_root = root.join("repo");
    let store = BookPipelineStore::for_test(&root);
    let (snapshot_executor, _) = collection_snapshot_executor_for_pdf(&root);
    let queued = queue_job_with_translation_intent_and_executor(
        &store,
        &snapshot_executor,
        real_collection_source(),
        MODE_CONVERT_THEN_TRANSLATE.into(),
        fast_translation_intent(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();
    let runner = CommandPipelineRunner::with_book_ocr_conversion_root(
        DurableCollectionChildExecutor::new(),
        worker_root,
    );
    run_job_with_handoff(
        &store,
        &runner,
        &FakeTranslationHandoffRunner,
        &queued.id,
        Some(&repo_root),
    )
    .unwrap();
    run_job_with_handoff(
        &store,
        &runner,
        &FakeTranslationHandoffRunner,
        &queued.id,
        Some(&repo_root),
    )
    .unwrap();
    let indexed = run_job_with_handoff(
        &store,
        &runner,
        &FakeTranslationHandoffRunner,
        &queued.id,
        Some(&repo_root),
    )
    .unwrap();
    let child = indexed
        .children
        .iter()
        .find(|child| child.source.selector.as_deref() == Some("PDFOK"))
        .unwrap();
    let markdown_path = child
        .artifacts
        .iter()
        .find(|artifact| artifact.kind == "markdown")
        .unwrap()
        .path
        .clone();
    fs::OpenOptions::new()
        .append(true)
        .open(&markdown_path)
        .unwrap()
        .write_all(b"\nchanged after index\n")
        .unwrap();

    let blocked = run_job_with_handoff(
        &store,
        &runner,
        &FakeTranslationHandoffRunner,
        &queued.id,
        Some(&repo_root),
    )
    .unwrap();

    let child = blocked
        .children
        .iter()
        .find(|child| child.source.selector.as_deref() == Some("PDFOK"))
        .unwrap();
    assert_eq!(stage_ref(child, "index").unwrap().status, STATUS_COMPLETED);
    assert_eq!(stage_ref(child, "handoff").unwrap().status, STATUS_BLOCKED);
    assert!(child
        .last_error
        .as_deref()
        .is_some_and(|error| error.contains("indexed Markdown")));
    assert!(!child
        .artifacts
        .iter()
        .any(|artifact| artifact.kind == "translation_source"));
    assert!(!repo_root.join("books").exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn durable_collection_mineru_route_uses_single_attachment_worker_adapter() {
    let root = temp_root("durable-collection-mineru-boundary");
    let store = BookPipelineStore::for_test(&root);
    let (snapshot_executor, _) = collection_snapshot_executor_for_pdf(&root);
    let queued = queue_job_with_translation_intent_and_executor(
        &store,
        &snapshot_executor,
        real_collection_source(),
        MODE_CONVERT_THEN_TRANSLATE.into(),
        fast_translation_intent(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();
    let mut child = queued
        .children
        .iter()
        .find(|child| child.source.selector.as_deref() == Some("PDFOK"))
        .unwrap()
        .clone();
    child.route[0].route_kind = "mineru".into();
    child.route[0].can_run = true;
    child.route[0].blocked_reason = None;

    let command = build_zotero_child_conversion_command_for_root(
        &child,
        &root.join("output"),
        &fake_full_worker_root(&root),
    )
    .unwrap();

    assert_eq!(command.label, ZOTERO_CONVERSION_COMMAND_LABEL);
    assert!(has_arg_pair(&command.args, "--attachment-key", "PDFOK"));
    assert!(command.args.iter().any(|arg| arg == "--force-mineru"));
    assert!(command.args.iter().any(|arg| arg == "--preserve-source"));
    assert!(!command.args.iter().any(|arg| arg.ends_with("mineru.py")));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn completed_worker_evidence_rejects_another_extraction_contract() {
    let root = temp_root("completed-worker-contract-mismatch");
    let store = BookPipelineStore::for_test(&root);
    let snapshot_executor = CollectionSnapshotExecutor::mixed_routes(&root);
    let queued = queue_job_with_translation_intent_and_executor(
        &store,
        &snapshot_executor,
        real_collection_source(),
        MODE_CONVERT_THEN_TRANSLATE.into(),
        fast_translation_intent(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();
    let child = queued
        .children
        .iter()
        .find(|child| child.source.selector.as_deref() == Some("DONE"))
        .unwrap();
    let markdown = root.join("legacy-completed.md");
    fs::write(
        &markdown,
        "---\nparent_item_key: \"PARENT4\"\n---\n\n# Legacy\n",
    )
    .unwrap();
    let payload = format!(
        "BOOK_PIPELINE_ATTACHMENT_EVIDENCE {}",
        serde_json::json!({
            "schemaVersion": ZOTERO_WORKER_ATTACHMENT_EVIDENCE_SCHEMA,
            "extractionContractVersion": "zotero-worker-extraction-legacy",
            "status": "already_completed",
            "route": "pdf-text",
            "pdfAttachmentKey": "DONE",
            "parentItemKey": "PARENT4",
            "sourceSha256": sha256_file(&root.join("zotero/DONE/done.pdf")).unwrap(),
            "markdownPath": display_path(&markdown),
            "markdownSha256": sha256_file(&markdown).unwrap(),
            "markdownAttachmentKey": "MARKDONE",
        })
    );
    let evidence = parse_zotero_worker_attachment_evidence(&payload, "DONE")
        .unwrap()
        .unwrap();

    let error = reused_markdown_artifact_from_evidence(child, &evidence).unwrap_err();

    assert!(error.contains("extraction contract"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn durable_mixed_collection_routes_and_isolates_attachment_outcomes() {
    let root = temp_root("durable-mixed-collection");
    let store = BookPipelineStore::for_test(&root);
    let snapshot_executor = CollectionSnapshotExecutor::mixed_routes(&root);
    let queued = queue_job_with_translation_intent_and_executor(
        &store,
        &snapshot_executor,
        real_collection_source(),
        MODE_CONVERT_THEN_TRANSLATE.into(),
        fast_translation_intent(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();
    let runner = CommandPipelineRunner::with_book_ocr_conversion_root(
        MixedDurableCollectionExecutor::new(&root),
        fake_zotero_worker_root(&root),
    );
    let repo_root = root.join("repo");

    for _ in 0..32 {
        let current = store
            .load()
            .unwrap()
            .jobs
            .into_iter()
            .find(|job| job.id == queued.id)
            .unwrap();
        if durable_collection_stage_to_run(&current).is_none() {
            break;
        }
        run_job_with_handoff(
            &store,
            &runner,
            &FakeTranslationHandoffRunner,
            &queued.id,
            Some(&repo_root),
        )
        .unwrap();
    }

    let finished = store
        .load()
        .unwrap()
        .jobs
        .into_iter()
        .find(|job| job.id == queued.id)
        .unwrap();
    assert!(durable_collection_stage_to_run(&finished).is_none());
    assert!(finished.collection_items.is_empty());
    assert_eq!(finished.children.len(), 7);
    for (key, expected_route) in [
        ("DIRECT", "direct_text"),
        ("SCAN", "remote_paddleocr"),
        ("MINERU", "mineru"),
    ] {
        let child = finished
            .children
            .iter()
            .find(|child| child.source.selector.as_deref() == Some(key))
            .unwrap();
        assert_eq!(child.route[0].route_kind, expected_route);
        assert_eq!(stage_ref(child, "route").unwrap().status, STATUS_COMPLETED);
        assert_eq!(
            stage_ref(child, "extract").unwrap().status,
            STATUS_COMPLETED
        );
        assert_eq!(stage_ref(child, "index").unwrap().status, STATUS_COMPLETED);
        assert_eq!(
            stage_ref(child, "handoff").unwrap().status,
            STATUS_COMPLETED
        );
    }
    let reused = finished
        .children
        .iter()
        .find(|child| child.source.selector.as_deref() == Some("DONE"))
        .unwrap();
    assert_eq!(reused.route[0].route_kind, "already_converted");
    assert_eq!(
        stage_ref(reused, "extract").unwrap().status,
        STATUS_COMPLETED
    );
    assert_eq!(stage_ref(reused, "index").unwrap().status, STATUS_COMPLETED);
    assert_eq!(
        stage_ref(reused, "handoff").unwrap().status,
        STATUS_COMPLETED
    );
    assert!(reused.artifacts.iter().any(|artifact| {
        artifact.kind == "markdown" && artifact.zotero_key.as_deref() == Some("MARKDONE")
    }));

    let failed = finished
        .children
        .iter()
        .find(|child| child.source.selector.as_deref() == Some("FAIL"))
        .unwrap();
    assert_eq!(stage_ref(failed, "route").unwrap().status, STATUS_COMPLETED);
    assert_eq!(stage_ref(failed, "extract").unwrap().status, STATUS_FAILED);
    assert_eq!(stage_ref(failed, "index").unwrap().status, STATUS_PENDING);
    assert_eq!(stage_ref(failed, "handoff").unwrap().status, STATUS_PENDING);
    let blocked = finished
        .children
        .iter()
        .find(|child| child.source.selector.as_deref() == Some("MISSING"))
        .unwrap();
    assert_eq!(stage_ref(blocked, "route").unwrap().status, STATUS_BLOCKED);
    assert_eq!(
        stage_ref(blocked, "extract").unwrap().status,
        STATUS_PENDING
    );
    let missing_credentials = finished
        .children
        .iter()
        .find(|child| child.source.selector.as_deref() == Some("NOPADDLE"))
        .unwrap();
    assert_eq!(
        stage_ref(missing_credentials, "route").unwrap().status,
        STATUS_BLOCKED
    );
    assert!(missing_credentials
        .last_error
        .as_deref()
        .is_some_and(|error| error.contains("PaddleOCR is unavailable")));
    assert_eq!(
        stage_ref(missing_credentials, "extract").unwrap().status,
        STATUS_PENDING
    );

    assert_eq!(finished.summary.total, 7);
    assert_eq!(finished.summary.ready, 4);
    assert_eq!(finished.summary.failed, 1);
    assert_eq!(finished.summary.blocked, 2);
    let calls = runner.executor.calls();
    for key in ["DIRECT", "SCAN", "MINERU", "DONE", "FAIL", "NOPADDLE"] {
        assert_eq!(
            calls
                .iter()
                .filter(|call| call.as_str() == format!("route:{key}"))
                .count(),
            1
        );
    }
    assert!(!calls.iter().any(|call| call == "extract:DONE"));
    assert!(!calls.iter().any(|call| call == "extract:NOPADDLE"));
    assert!(calls.iter().any(|call| call == "index:PARENT4"));
    assert_eq!(
        finished.membership.as_ref().unwrap().snapshot_sha256,
        queued.membership.as_ref().unwrap().snapshot_sha256
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn durable_collection_restart_retries_only_interrupted_child_stages() {
    let root = temp_root("durable-collection-targeted-recovery");
    let store = BookPipelineStore::for_test_with_owner(&root, "worker-before-restart");
    let snapshot_executor = CollectionSnapshotExecutor::recovery_routes(&root);
    let queued = queue_job_with_translation_intent_and_executor(
        &store,
        &snapshot_executor,
        real_collection_source(),
        MODE_CONVERT_THEN_TRANSLATE.into(),
        fast_translation_intent(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();
    let frozen_membership = queued.membership.clone().unwrap();
    let runner = CommandPipelineRunner::with_book_ocr_conversion_root(
        MixedDurableCollectionExecutor::new(&root),
        fake_zotero_worker_root(&root),
    );
    let repo_root = root.join("repo");
    let run_next = |store: &BookPipelineStore| {
        run_job_with_handoff(
            store,
            &runner,
            &FakeTranslationHandoffRunner,
            &queued.id,
            Some(&repo_root),
        )
        .unwrap()
    };
    let mark_interrupted = |store: &BookPipelineStore, key: &str, stage_id: &str| {
        let mut state = store.load().unwrap();
        let job = state
            .jobs
            .iter_mut()
            .find(|job| job.id == queued.id)
            .unwrap();
        let child_index = job
            .children
            .iter()
            .position(|child| child.source.selector.as_deref() == Some(key))
            .unwrap();
        let index_input_hashes = (stage_id == "index").then(|| {
            let child = &job.children[child_index];
            let markdown = child
                .artifacts
                .iter()
                .find(|artifact| artifact.kind == "markdown")
                .unwrap();
            runner
                .index_input(job, child, markdown, &store.job_output_dir(&queued.id))
                .unwrap()
                .hashes()
        });
        let child = &mut job.children[child_index];
        if let Some(input_hashes) = index_input_hashes {
            stage_mut(child, "index").unwrap().input_hashes = input_hashes;
        }
        start_stage(child, stage_id, store.execution_owner().unwrap());
        child.attempts = child.attempts.saturating_add(1);
        derive_job(job);
        store.save(&state).unwrap();
    };

    for _ in 0..4 {
        run_next(&store);
    }
    for _ in 0..2 {
        run_next(&store);
    }
    mark_interrupted(&store, "SCAN", "index");
    run_next(&store);
    mark_interrupted(&store, "MINERU", "extract");
    for _ in 0..3 {
        run_next(&store);
    }
    mark_interrupted(&store, "DONE", "handoff");

    let before_restart = store.load().unwrap();
    let job_before_restart = before_restart
        .jobs
        .iter()
        .find(|job| job.id == queued.id)
        .unwrap();
    assert_eq!(
        job_before_restart.membership.as_ref(),
        Some(&frozen_membership)
    );
    assert_eq!(
        stage_ref(
            job_before_restart
                .children
                .iter()
                .find(|child| child.source.selector.as_deref() == Some("DIRECT"))
                .unwrap(),
            "handoff",
        )
        .unwrap()
        .status,
        STATUS_COMPLETED
    );
    drop(store);

    let restarted = BookPipelineStore::for_test_with_owner(&root, "worker-after-restart");
    let recovered = restarted.load().unwrap();
    let recovered_revision = recovered.revision;
    let recovered_job = recovered
        .jobs
        .iter()
        .find(|job| job.id == queued.id)
        .unwrap();
    for (key, stage_id) in [
        ("SCAN", "index"),
        ("MINERU", "extract"),
        ("DONE", "handoff"),
    ] {
        let stage = stage_ref(
            recovered_job
                .children
                .iter()
                .find(|child| child.source.selector.as_deref() == Some(key))
                .unwrap(),
            stage_id,
        )
        .unwrap();
        assert_eq!(stage.status, STATUS_FAILED);
        assert_eq!(stage.attempt, 1);
        assert_eq!(stage.safe_error.as_ref().unwrap().code, "interrupted");
    }
    assert_eq!(restarted.load().unwrap().revision, recovered_revision);

    for _ in 0..6 {
        retry_job_with_handoff(
            &restarted,
            &runner,
            &FakeTranslationHandoffRunner,
            &queued.id,
            Some(&repo_root),
        )
        .unwrap();
    }

    let finished = restarted
        .load()
        .unwrap()
        .jobs
        .into_iter()
        .find(|job| job.id == queued.id)
        .unwrap();
    assert_eq!(finished.membership.as_ref(), Some(&frozen_membership));
    assert!(finished.collection_items.is_empty());
    for key in ["DIRECT", "SCAN", "MINERU", "DONE"] {
        let child = finished
            .children
            .iter()
            .find(|child| child.source.selector.as_deref() == Some(key))
            .unwrap();
        assert_eq!(
            stage_ref(child, "handoff").unwrap().status,
            STATUS_COMPLETED
        );
    }
    let stage_attempt = |key: &str, stage_id: &str| {
        stage_ref(
            finished
                .children
                .iter()
                .find(|child| child.source.selector.as_deref() == Some(key))
                .unwrap(),
            stage_id,
        )
        .unwrap()
        .attempt
    };
    assert_eq!(stage_attempt("DIRECT", "handoff"), 1);
    assert_eq!(stage_attempt("SCAN", "extract"), 1);
    assert_eq!(stage_attempt("SCAN", "index"), 2);
    assert_eq!(stage_attempt("MINERU", "extract"), 2);
    assert_eq!(stage_attempt("MINERU", "index"), 1);
    assert_eq!(stage_attempt("DONE", "extract"), 1);
    assert_eq!(stage_attempt("DONE", "handoff"), 2);
    let calls = runner.executor.calls();
    assert_eq!(
        calls
            .iter()
            .filter(|call| *call == "extract:DIRECT")
            .count(),
        1
    );
    assert_eq!(
        calls.iter().filter(|call| *call == "extract:SCAN").count(),
        1
    );
    assert_eq!(
        calls
            .iter()
            .filter(|call| *call == "extract:MINERU")
            .count(),
        1
    );
    assert!(!calls.iter().any(|call| call == "extract:DONE"));
    assert_eq!(*snapshot_executor.calls.lock().unwrap(), 1);

    let revision_before_repeat = restarted.load().unwrap().revision;
    let calls_before_repeat = runner.executor.calls();
    let repeated = retry_job_with_handoff(
        &restarted,
        &runner,
        &FakeTranslationHandoffRunner,
        &queued.id,
        Some(&repo_root),
    )
    .unwrap();
    assert_eq!(restarted.load().unwrap().revision, revision_before_repeat);
    assert_eq!(runner.executor.calls(), calls_before_repeat);
    assert_eq!(repeated.membership.as_ref(), Some(&frozen_membership));
    assert!(repeated.children.iter().all(|child| {
        stage_ref(child, "handoff").is_some_and(|stage| stage.status == STATUS_COMPLETED)
    }));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn durable_collection_user_run_and_retry_complete_the_searchable_end_to_end_chain() {
    let root = temp_root("durable-collection-end-to-end");
    let store = BookPipelineStore::for_test_with_owner(&root, "worker-before-restart");
    let snapshot_executor = CollectionSnapshotExecutor::recovery_routes(&root);
    let queued = queue_job_with_translation_intent_and_executor(
        &store,
        &snapshot_executor,
        real_collection_source(),
        MODE_CONVERT_THEN_TRANSLATE.into(),
        fast_translation_intent(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();
    let frozen_membership = queued.membership.clone().unwrap();
    let runner = CommandPipelineRunner::with_book_ocr_conversion_root(
        MixedDurableCollectionExecutor::failing_index_once(&root, "PARENT2"),
        fake_zotero_worker_root(&root),
    );
    let repo_root = root.join("repo");

    let first_run = run_job_to_quiescence_with_handoff(
        &store,
        &runner,
        &FakeTranslationHandoffRunner,
        &queued.id,
        Some(&repo_root),
    )
    .unwrap();

    assert_eq!(*snapshot_executor.calls.lock().unwrap(), 1);
    assert_eq!(first_run.membership.as_ref(), Some(&frozen_membership));
    assert!(first_run.collection_items.is_empty());
    assert_eq!(first_run.summary.total, 4);
    assert_eq!(first_run.summary.failed, 1);
    for key in ["DIRECT", "MINERU", "DONE"] {
        let child = first_run
            .children
            .iter()
            .find(|child| child.source.selector.as_deref() == Some(key))
            .unwrap();
        assert_eq!(
            stage_ref(child, "handoff").unwrap().status,
            STATUS_COMPLETED
        );
    }
    let scan = first_run
        .children
        .iter()
        .find(|child| child.source.selector.as_deref() == Some("SCAN"))
        .unwrap();
    assert_eq!(stage_ref(scan, "extract").unwrap().status, STATUS_COMPLETED);
    assert_eq!(stage_ref(scan, "index").unwrap().status, STATUS_FAILED);
    assert_eq!(stage_ref(scan, "handoff").unwrap().status, STATUS_PENDING);
    assert_eq!(runner.executor.indexed_sha256("PARENT2"), None);
    drop(store);

    let restarted = BookPipelineStore::for_test_with_owner(&root, "worker-after-restart");
    let completed = retry_job_to_quiescence_with_handoff(
        &restarted,
        &runner,
        &FakeTranslationHandoffRunner,
        &queued.id,
        Some(&repo_root),
    )
    .unwrap();

    assert_eq!(completed.membership.as_ref(), Some(&frozen_membership));
    assert!(completed.collection_items.is_empty());
    assert_eq!(completed.summary.total, 4);
    assert_eq!(completed.summary.failed, 0);
    assert_eq!(completed.summary.blocked, 0);
    for (key, parent) in [
        ("DIRECT", "PARENT1"),
        ("SCAN", "PARENT2"),
        ("MINERU", "PARENT3"),
        ("DONE", "PARENT4"),
    ] {
        let child = completed
            .children
            .iter()
            .find(|child| child.source.selector.as_deref() == Some(key))
            .unwrap();
        assert_eq!(
            stage_ref(child, "handoff").unwrap().status,
            STATUS_COMPLETED
        );
        let evidence = stage_ref(child, "index")
            .unwrap()
            .index_evidence
            .as_ref()
            .unwrap();
        assert_eq!(
            runner.executor.indexed_sha256(parent).as_deref(),
            Some(evidence.source_sha256.as_str()),
            "fake zfulltext query surface must immediately expose the indexed artifact"
        );
        assert!(child
            .artifacts
            .iter()
            .any(|artifact| artifact.kind == "translation_source"));
    }
    let calls = runner.executor.calls();
    assert_eq!(
        calls.iter().filter(|call| *call == "extract:SCAN").count(),
        1
    );
    assert_eq!(
        calls.iter().filter(|call| *call == "index:PARENT2").count(),
        2
    );
    assert_eq!(*snapshot_executor.calls.lock().unwrap(), 1);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn store_rejects_running_stage_with_incomplete_prerequisite() {
    let root = temp_root("stage-prerequisite-validation");
    let store = BookPipelineStore::for_test(&root);
    let job = queue_job(
        &store,
        fake_source(None),
        MODE_CONVERT_THEN_TRANSLATE.into(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();
    let mut state = store.load().unwrap();
    let child = &mut state
        .jobs
        .iter_mut()
        .find(|stored| stored.id == job.id)
        .unwrap()
        .children[0];
    start_stage(child, "handoff", store.execution_owner().unwrap());

    let error = store.save(&state).unwrap_err();

    assert!(error.contains("incomplete prerequisite"));
    assert!(error.contains("handoff"));
    let _ = fs::remove_dir_all(root);
}

// The runner's output lives only in memory until the save below the handoff
// start. A handoff that could not start used to return through `?` and take
// the whole conversion with it, leaving the extract stage `running` on disk,
// so retrying the handoff meant re-running the OCR.
#[test]
fn a_handoff_that_cannot_start_keeps_the_extraction_it_just_produced() {
    let root = temp_root("handoff-early-return");
    let repo_root = root.join("repo");
    let store = MemoryStateStore::new(&root);
    let job = queue_job(
        &store,
        fake_source(None),
        MODE_CONVERT_THEN_TRANSLATE.into(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();

    // A handoff stage left running is a state `mark_handoff_running` refuses
    // to start from, which is what puts the job on this path.
    let mut state = store.load().unwrap();
    let child = &mut state.jobs[0].children[0];
    ensure_translation_stages(child, false);
    stage_mut(child, "handoff").unwrap().status = STATUS_RUNNING.into();
    store.save(&state).unwrap();

    let finished = run_job_with_handoff(
        &store,
        &ArtifactFixtureRunner,
        &FakeTranslationHandoffRunner,
        &job.id,
        Some(&repo_root),
    )
    .unwrap();

    let stored = store.load().unwrap().jobs[0].clone();
    assert!(
        stored
            .artifacts
            .iter()
            .any(|artifact| artifact.kind == "markdown"),
        "the conversion the runner just produced must survive: {:?}",
        stored.artifacts
    );
    assert!(stored.output_dir.is_some(), "output_dir was dropped");
    assert_eq!(stored.artifacts, finished.artifacts);
    let child = &stored.children[0];
    assert_eq!(
        stage_ref(child, "extract").unwrap().status,
        STATUS_COMPLETED,
        "the extraction must not be left running"
    );
    assert_eq!(stage_ref(child, "handoff").unwrap().status, STATUS_FAILED);
    assert_eq!(
        stored.last_error.as_deref(),
        Some("No completed extraction is ready for translation handoff.")
    );

    // Retrying the handoff alone has to work from here: this entry point
    // takes no pipeline runner, so it cannot re-run the extraction.
    let handed_off = handoff_job_markdown_with_runner(
        &store,
        &job.id,
        None,
        &repo_root,
        &FakeTranslationHandoffRunner,
    )
    .unwrap();
    let child = &handed_off.children[0];
    assert_eq!(
        stage_ref(child, "extract").unwrap().status,
        STATUS_COMPLETED
    );
    assert_eq!(
        stage_ref(child, "handoff").unwrap().status,
        STATUS_COMPLETED
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn store_rejects_completed_stage_regression_without_invalidation() {
    let root = temp_root("invalid-stage-transition");
    let store = BookPipelineStore::for_test(&root);
    let job = queue_job(
        &store,
        fake_source(None),
        "conversion_only".into(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();

    let set_extract_status = |state: &mut BookPipelineState, status: &str| {
        let stored = state
            .jobs
            .iter_mut()
            .find(|stored| stored.id == job.id)
            .unwrap();
        if status == STATUS_RUNNING {
            start_stage(
                &mut stored.children[0],
                "extract",
                store.execution_owner().unwrap(),
            );
        } else {
            set_stage_status(&mut stored.children[0], "extract", status, None);
        }
    };

    let mut running = store.load().unwrap();
    set_extract_status(&mut running, STATUS_RUNNING);
    store.save(&running).unwrap();
    let mut completed = store.load().unwrap();
    set_extract_status(&mut completed, STATUS_COMPLETED);
    store.save(&completed).unwrap();
    let mut regressed = store.load().unwrap();
    set_extract_status(&mut regressed, STATUS_RUNNING);

    let error = store.save(&regressed).unwrap_err();

    assert!(error.contains("Invalid Book Pipeline stage transition"));
    assert!(error.contains("extract"));
    assert!(error.contains("completed -> running"));
    let recovered = store.load().unwrap();
    let extract = recovered
        .jobs
        .iter()
        .find(|stored| stored.id == job.id)
        .unwrap()
        .children[0]
        .stages
        .iter()
        .find(|stage| stage.stage_id == "extract")
        .unwrap();
    assert_eq!(extract.status, STATUS_COMPLETED);

    let mut invalidated = store.load().unwrap();
    let extract = invalidated
        .jobs
        .iter_mut()
        .find(|stored| stored.id == job.id)
        .unwrap()
        .children[0]
        .stages
        .iter_mut()
        .find(|stage| stage.stage_id == "extract")
        .unwrap();
    extract.contract_version = "book-pipeline-job-v3-test".into();
    extract.status = STATUS_READY.into();
    store.save(&invalidated).unwrap();
    let recovered = store.load().unwrap();
    assert_eq!(
        recovered
            .jobs
            .iter()
            .find(|stored| stored.id == job.id)
            .unwrap()
            .children[0]
            .stages
            .iter()
            .find(|stage| stage.stage_id == "extract")
            .unwrap()
            .status,
        STATUS_READY
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn failed_and_blocked_children_do_not_regress_parent_to_pending() {
    let summary = BookPipelineStatusSummary {
        total: 2,
        failed: 1,
        blocked: 1,
        ..BookPipelineStatusSummary::default()
    };

    assert_eq!(aggregate_parent_status(&summary), STATUS_BLOCKED);
}

#[test]
fn public_state_reports_stage_and_unit_progress() {
    let root = temp_root("public-progress-contract");
    let store = BookPipelineStore::for_test(&root);
    let job = queue_job(
        &store,
        fake_source(None),
        "conversion_only".into(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();

    let queued = store
        .load()
        .unwrap()
        .jobs
        .into_iter()
        .find(|stored| stored.id == job.id)
        .unwrap();

    assert_eq!(queued.progress.stage_total, 3);
    assert_eq!(queued.progress.stage_completed, 2);
    assert_eq!(queued.progress.percent, 66);
    assert_eq!(queued.progress.active_stage_id, "extract");
    assert!(queued.progress.unit_summary.is_none());
    let serialized = serde_json::to_value(&queued).unwrap();
    assert_eq!(serialized["progress"]["stageTotal"], 3);
    assert_eq!(serialized["progress"]["activeStageId"], "extract");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn public_state_overlays_live_worker_progress_without_persisting_it() {
    let root = temp_root("live-worker-progress-contract");
    let store = BookPipelineStore::for_test(&root);
    let job = queue_job(
        &store,
        fake_source(None),
        "conversion_only".into(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();
    let mut state = store.load().unwrap();
    let stored = state
        .jobs
        .iter_mut()
        .find(|stored| stored.id == job.id)
        .unwrap();
    let child = stored.children.first_mut().unwrap();
    start_stage(child, "extract", store.execution_owner().unwrap());
    stage_mut(child, "extract").unwrap().started_at = Some("2026-07-29T11:00:00Z".into());
    child.status = STATUS_RUNNING.into();
    derive_job(stored);
    store.save(&state).unwrap();

    let output_dir = store.job_output_dir(&job.id);
    fs::create_dir_all(&output_dir).unwrap();
    fs::write(
        output_dir.join(LIVE_PROGRESS_FILE),
        r#"{
          "schema":"book-pipeline-progress-v1",
          "stageId":"extract",
          "completed":37,
          "total":100,
          "unitKind":"pages",
          "phase":"extracting",
          "activityAt":"2026-07-29T12:00:00Z"
        }"#,
    )
    .unwrap();

    let observed = load_state_with_live_progress(&store).unwrap();
    let progress = observed.jobs[0].progress.operation.as_ref().unwrap();
    assert_eq!(progress.completed, 37);
    assert_eq!(progress.total, Some(100));
    assert_eq!(progress.unit_kind, "pages");
    assert_eq!(
        observed.jobs[0]
            .progress
            .unit_summary
            .as_ref()
            .unwrap()
            .completed,
        37
    );
    assert!(
        store.load().unwrap().jobs[0].progress.operation.is_none(),
        "ephemeral worker progress must not be written into durable jobs.json"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn live_progress_older_than_the_current_attempt_is_ignored() {
    let stage = BookPipelineStage {
        stage_id: "translate".into(),
        status: STATUS_RUNNING.into(),
        started_at: Some("2026-07-29T12:00:00Z".into()),
        ..BookPipelineStage::default()
    };
    let progress = BookPipelineOperationProgress {
        stage_id: "translate".into(),
        completed: 99,
        total: Some(100),
        unit_kind: "chapters".into(),
        phase: "translating".into(),
        activity_at: "2026-07-29T11:59:59Z".into(),
        ..BookPipelineOperationProgress::default()
    };

    assert!(!live_progress_matches_stage(&stage, &progress));
}

#[test]
fn terminal_webhook_is_safe_deterministic_and_idempotent() {
    let root = temp_root("terminal-webhook-contract");
    let store = BookPipelineStore::for_test(&root);
    let mut source = fake_source(None);
    source.title = Some("private title must not leave the app".into());
    source.path = Some("/private/library/secret.md".into());
    let job = queue_job(
        &store,
        source,
        "conversion_only".into(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();
    run_job(&store, &SystemPipelineRunner, &job.id).unwrap();
    let sink = RecordingNotificationSink::default();

    let first = dispatch_terminal_notification(&store, &sink, &job.id).unwrap();
    let second = dispatch_terminal_notification(&store, &sink, &job.id).unwrap();

    assert_eq!(first.notification_deliveries.len(), 1);
    assert_eq!(second.notification_deliveries.len(), 1);
    let events = sink.events.lock().unwrap();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.status, STATUS_COMPLETED);
    assert_eq!(event.progress.percent, 100);
    assert_eq!(event.event_id, first.notification_deliveries[0].event_id);
    let payload = serde_json::to_string(event).unwrap();
    assert!(!payload.contains("private title"));
    assert!(!payload.contains("/private/library"));
    assert!(!payload.contains("lastError"));
    assert!(!payload.contains("logSummary"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn terminal_webhook_progress_excludes_local_unit_failure_details() {
    let root = temp_root("terminal-webhook-local-failures");
    let store = BookPipelineStore::for_test(&root);
    let mut job = queue_job(
        &store,
        fake_source(None),
        "conversion_only".into(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();
    job.progress.unit_summary = Some(BookPipelineUnitSummary {
        total: 2,
        completed: 1,
        failed: 1,
        failures: vec![BookPipelineUnitFailure {
            unit_id: "private-chapter-name".into(),
            code: "provider_timeout".into(),
            retryable: true,
        }],
        ..BookPipelineUnitSummary::default()
    });

    let event = terminal_event(&job);

    let summary = event.progress.unit_summary.as_ref().unwrap();
    assert_eq!(
        (summary.total, summary.completed, summary.failed),
        (2, 1, 1)
    );
    assert!(summary.failures.is_empty());
    let payload = serde_json::to_string(&event).unwrap();
    assert!(!payload.contains("private-chapter-name"));
    assert!(!payload.contains("provider_timeout"));
    let _ = fs::remove_dir_all(root);
}

// ADR 0002 promises one webhook per terminal outcome. Folding `updated_at`
// and `attempts` into the event id delivered one per (outcome, timestamp),
// so a job that reached the same terminal status again — retried and failed
// again, or simply touched while terminal — notified a second time.
#[test]
fn reaching_the_same_terminal_status_again_delivers_one_webhook() {
    let root = temp_root("terminal-webhook-restate");
    // A memory store so the terminal status can be restated directly; the
    // durable store re-derives it from the children on every save.
    let store = MemoryStateStore::new(&root);
    let job = queue_job(
        &store,
        fake_source(None),
        "conversion_only".into(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();
    run_job(&store, &ArtifactFixtureRunner, &job.id).unwrap();
    let sink = RecordingNotificationSink::default();

    dispatch_terminal_notification(&store, &sink, &job.id).unwrap();

    // Reaching the terminal status again moves the clock and the attempt
    // counter; neither may mint a second event.
    let mut state = store.load().unwrap();
    let stored = state
        .jobs
        .iter_mut()
        .find(|stored| stored.id == job.id)
        .unwrap();
    stored.attempts += 1;
    stored.updated_at = "2026-07-26T09:00:00Z".into();
    store.save(&state).unwrap();

    let second = dispatch_terminal_notification(&store, &sink, &job.id).unwrap();

    assert_eq!(
        sink.events.lock().unwrap().len(),
        1,
        "the same terminal outcome must notify once"
    );
    assert_eq!(second.notification_deliveries.len(), 1);

    // A different terminal outcome is still its own event.
    let mut state = store.load().unwrap();
    let stored = state
        .jobs
        .iter_mut()
        .find(|stored| stored.id == job.id)
        .unwrap();
    stored.status = STATUS_FAILED.into();
    store.save(&state).unwrap();

    let failed = dispatch_terminal_notification(&store, &sink, &job.id).unwrap();

    let events = sink.events.lock().unwrap();
    assert_eq!(events.len(), 2);
    assert_eq!(events[1].status, STATUS_FAILED);
    assert_ne!(events[0].event_id, events[1].event_id);
    assert_eq!(failed.notification_deliveries.len(), 2);
    drop(events);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn webhook_config_reads_only_the_requested_dotenv_value() {
    let raw = "# private config\nOTHER_SETTING=ignored\nexport BOOK_PIPELINE_WEBHOOK_URL='https://localhost/hooks/books'\n";

    assert_eq!(
        dotenv_value(raw, "BOOK_PIPELINE_WEBHOOK_URL").as_deref(),
        Some("https://localhost/hooks/books")
    );
    assert_eq!(dotenv_value(raw, "MISSING"), None);
}

#[test]
fn concurrent_saves_reject_one_stale_revision_and_keep_valid_json() {
    let root = temp_root("concurrent-save-protection");
    let store = std::sync::Arc::new(BookPipelineStore::for_test(&root));
    let job = queue_job(
        store.as_ref(),
        fake_source(None),
        "conversion_only".into(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();
    let mut left = store.load().unwrap();
    let mut right = store.load().unwrap();
    let starting_revision = left.revision;
    left.jobs
        .iter_mut()
        .find(|stored| stored.id == job.id)
        .unwrap()
        .log_summary
        .push("left writer".into());
    right
        .jobs
        .iter_mut()
        .find(|stored| stored.id == job.id)
        .unwrap()
        .log_summary
        .push("right writer".into());
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let left_store = std::sync::Arc::clone(&store);
    let left_barrier = std::sync::Arc::clone(&barrier);
    let left_writer = std::thread::spawn(move || {
        left_barrier.wait();
        left_store.save(&left)
    });
    let right_store = std::sync::Arc::clone(&store);
    let right_barrier = std::sync::Arc::clone(&barrier);
    let right_writer = std::thread::spawn(move || {
        right_barrier.wait();
        right_store.save(&right)
    });

    let results = [left_writer.join().unwrap(), right_writer.join().unwrap()];

    assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
    let conflict = results
        .iter()
        .find_map(|result| result.as_ref().err())
        .unwrap();
    assert!(conflict.contains("Book Pipeline state changed concurrently"));
    let persisted_text = fs::read_to_string(&store.state_path).unwrap();
    let persisted: BookPipelineState = serde_json::from_str(&persisted_text).unwrap();
    assert_eq!(persisted.revision, starting_revision + 1);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn fake_job_records_failure_and_retry_success() {
    let root = temp_root("fake-retry");
    let store = BookPipelineStore::for_test(&root);
    let source = fake_source(Some("fail_once"));

    let job = queue_job(
        &store,
        source,
        "conversion_only".into(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();
    let failed = run_job(&store, &SystemPipelineRunner, &job.id).unwrap();
    assert_eq!(failed.status, STATUS_FAILED);
    assert!(failed.last_error.is_some());
    assert_eq!(failed.attempts, 1);

    let completed = run_job(&store, &SystemPipelineRunner, &job.id).unwrap();
    assert_eq!(completed.status, STATUS_COMPLETED);
    assert_eq!(completed.attempts, 2);
    assert!(completed
        .artifacts
        .iter()
        .any(|artifact| artifact.kind == "markdown" && artifact.sha256.is_some()));

    let recovered = store.load().unwrap();
    assert_eq!(recovered.jobs[0].status, STATUS_COMPLETED);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn completed_job_rejects_duplicate_runner_execution() {
    let root = temp_root("completed-job-no-duplicate-run");
    let store = BookPipelineStore::for_test(&root);
    let job = queue_job(
        &store,
        fake_source(None),
        "conversion_only".into(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();
    let completed = run_job(&store, &SystemPipelineRunner, &job.id).unwrap();
    assert_eq!(completed.status, STATUS_COMPLETED);

    let error = run_job(&store, &SystemPipelineRunner, &job.id).unwrap_err();

    assert!(error.contains("No eligible extraction stage"));
    let recovered = store.load().unwrap();
    let recovered = recovered
        .jobs
        .iter()
        .find(|stored| stored.id == job.id)
        .unwrap();
    assert_eq!(recovered.attempts, 1);
    assert_eq!(recovered.status, STATUS_COMPLETED);
    let _ = fs::remove_dir_all(root);
}

fn zotero_route(route_kind: &str) -> BookPipelineRouteItem {
    BookPipelineRouteItem {
        id: "ATTACH1".into(),
        title: "Fixture attachment".into(),
        source_kind: "zotero_attachment".into(),
        source_ref: "ATTACH1".into(),
        route_kind: route_kind.into(),
        can_run: true,
        blocked_reason: None,
        summary: "fixture route".into(),
        route_override: None,
    }
}

fn assert_runs_in_the_ocr_workspace(command: &RunnerCommand, script: &str) {
    assert_eq!(
        command.program,
        PathBuf::from("uv"),
        "{}: a bare interpreter only finds PyMuPDF where it happens to be installed globally",
        command.label
    );
    assert_eq!(
        command.args[..4],
        [
            "run".to_string(),
            "--package".to_string(),
            "ocr".to_string(),
            "python".to_string(),
        ],
        "{} must resolve its imports from the workspace venv",
        command.label
    );
    assert!(
        command.args[4].ends_with(script),
        "{} should run {script}, got {}",
        command.label,
        command.args[4]
    );
}

// The OCR line was the one pipeline stage still spawning a bare interpreter,
// so its imports came from whatever the machine happened to have installed.
#[test]
fn every_ocr_entry_point_runs_through_the_workspace_venv() {
    let root = temp_root("ocr-workspace-venv");
    let input = root.join("input");
    let output = root.join("output");
    fs::create_dir_all(&input).unwrap();
    let worker_root = fake_full_worker_root(&root);
    fake_wrapper_root(&root);
    let store = BookPipelineStore::for_test(&root);
    let job = queue_job(
        &store,
        local_pdf_source(&input),
        "conversion_only".into(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();

    assert_runs_in_the_ocr_workspace(
        &build_local_pdf_folder_command_for_root(&job, &output, &worker_root).unwrap(),
        "pdf_to_html_paddleocr.py",
    );
    assert_runs_in_the_ocr_workspace(
        &build_zotero_conversion_command_for_source(
            &fake_direct_zotero_source(),
            &zotero_route("direct_text"),
            0,
            &output,
            &worker_root,
        )
        .unwrap(),
        "zotero_llm_worker.py",
    );
    let mineru_command = build_zotero_conversion_command_for_source(
        &fake_direct_zotero_source(),
        &zotero_route("mineru"),
        0,
        &output,
        &worker_root,
    )
    .unwrap();
    assert_runs_in_the_ocr_workspace(&mineru_command, "zotero_llm_worker.py");
    assert!(mineru_command
        .args
        .iter()
        .any(|arg| arg == "--force-mineru"));
    assert_runs_in_the_ocr_workspace(
        &build_zotero_discovery_command_for_root(&fake_direct_zotero_source(), 5, &worker_root)
            .unwrap(),
        "zotero_llm_worker.py",
    );

    fs::remove_dir_all(&root).ok();
}

// `~/BiblioSmith` is a guess, not a promise. Handing a missing directory to
// the runner as its cwd only produced an errno about a path the user never
// picked, so the check has to name the settings that fix it instead.
#[test]
fn missing_repo_root_names_the_settings_that_fix_it() {
    let missing = temp_root("repo-root-absent");
    let error = existing_repo_root(missing.clone()).unwrap_err();

    assert!(error.contains(&display_path(&missing)), "{error}");
    assert!(error.contains("设置"), "{error}");
    assert!(error.contains("BIBLIOSMITH_HOME"), "{error}");

    fs::create_dir_all(&missing).unwrap();
    assert_eq!(existing_repo_root(missing.clone()).unwrap(), missing);
    fs::remove_dir_all(&missing).ok();
}

#[test]
fn local_pdf_runner_command_uses_existing_wrapper_contract() {
    let root = temp_root("local-pdf-command");
    let input = root.join("input");
    let output = root.join("output");
    fs::create_dir_all(&input).unwrap();
    let wrapper_root = fake_wrapper_root(&root);
    let wrapper_script = wrapper_root
        .join("scripts")
        .join("pdf_to_html_paddleocr.py");
    let source = local_pdf_source(&input);
    let store = BookPipelineStore::for_test(&root);
    let job = queue_job(
        &store,
        source,
        "conversion_only".into(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();

    let command = build_local_pdf_folder_command_for_root(&job, &output, &wrapper_root).unwrap();

    assert_eq!(command.kind, RunnerCommandKind::Process);
    assert_eq!(command.label, "local PDF conversion wrapper");
    assert_eq!(command.cwd, Some(wrapper_root));
    assert_eq!(command.output_dir, output);
    assert_eq!(command.program, PathBuf::from("uv"));
    assert_eq!(
        command.args[..5],
        [
            "run".to_string(),
            "--package".to_string(),
            "ocr".to_string(),
            "python".to_string(),
            display_path(&wrapper_script),
        ]
    );
    assert!(has_arg_pair(
        &command.args,
        "--input-dir",
        &display_path(&input)
    ));
    assert!(has_arg_pair(
        &command.args,
        "--output-dir",
        &display_path(&command.output_dir)
    ));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn local_pdf_folder_forced_to_mineru_uses_precision_batch_client() {
    let root = temp_root("local-pdf-mineru-batch-command");
    let input = root.join("input");
    let output = root.join("output");
    fs::create_dir_all(&input).unwrap();
    fs::write(input.join("one.pdf"), "%PDF fixture one").unwrap();
    fs::write(input.join("two.pdf"), "%PDF fixture two").unwrap();
    let wrapper_root = fake_wrapper_root(&root);
    let mineru_script = wrapper_root.join("mineru.py");
    fs::write(&mineru_script, "print('mineru fixture')\n").unwrap();
    let store = BookPipelineStore::for_test(&root);
    let job = queue_job(
        &store,
        local_pdf_source(&input),
        "conversion_only".into(),
        BookPipelinePreviewConfig {
            has_paddleocr_credentials: false,
            has_mineru_credentials: true,
            route_overrides: BTreeMap::from([
                ("local-pdf-1".into(), "mineru".into()),
                ("local-pdf-2".into(), "mineru".into()),
            ]),
        },
    )
    .unwrap();

    let command = build_local_pdf_folder_command_for_root(&job, &output, &wrapper_root).unwrap();

    assert_eq!(command.label, "MinerU Precision batch");
    assert_runs_in_the_ocr_workspace(&command, "mineru.py");
    assert!(command.args.iter().any(|arg| arg == &display_path(&input)));
    assert!(has_arg_pair(&command.args, "--mode", "batch"));
    assert!(has_arg_pair(&command.args, "--model-version", "vlm"));
    assert!(has_arg_pair(
        &command.args,
        "--output-dir",
        &display_path(&output)
    ));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn local_pdf_folder_rejects_a_mixed_mineru_and_paddle_batch() {
    let root = temp_root("local-pdf-mixed-ocr-command");
    let input = root.join("input");
    let output = root.join("output");
    fs::create_dir_all(&input).unwrap();
    fs::write(input.join("one.pdf"), "%PDF fixture one").unwrap();
    fs::write(input.join("two.pdf"), "%PDF fixture two").unwrap();
    let wrapper_root = fake_wrapper_root(&root);
    fs::write(wrapper_root.join("mineru.py"), "print('mineru fixture')\n").unwrap();
    let store = BookPipelineStore::for_test(&root);
    let job = queue_job(
        &store,
        local_pdf_source(&input),
        "conversion_only".into(),
        BookPipelinePreviewConfig {
            has_paddleocr_credentials: true,
            has_mineru_credentials: true,
            route_overrides: BTreeMap::from([("local-pdf-1".into(), "mineru".into())]),
        },
    )
    .unwrap();

    let error = build_local_pdf_folder_command_for_root(&job, &output, &wrapper_root).unwrap_err();

    assert!(
        error.contains("cannot mix MinerU and non-MinerU"),
        "{error}"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn local_pdf_runner_contract_records_wrapper_artifacts() {
    let root = temp_root("local-pdf-wrapper-artifacts");
    let input = root.join("input");
    fs::create_dir_all(&input).unwrap();
    fs::write(input.join("sample.pdf"), "%PDF fixture").unwrap();
    let wrapper_root = fake_wrapper_root(&root);
    let store = BookPipelineStore::for_test(&root);
    let job = queue_job(
        &store,
        local_pdf_source(&input),
        "conversion_only".into(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();

    let completed = run_job(
        &store,
        &CommandPipelineRunner::with_book_ocr_conversion_root(
            LocalPdfFixtureExecutor,
            wrapper_root,
        ),
        &job.id,
    )
    .unwrap();

    assert_eq!(completed.status, STATUS_COMPLETED);
    assert_eq!(completed.current_step, "Completed");
    assert!(completed.last_error.is_none());
    assert!(completed.output_dir.is_some());
    for kind in ["markdown", "html", "epub", "metadata", "index"] {
        assert!(completed
            .artifacts
            .iter()
            .any(|artifact| artifact.kind == kind && artifact.sha256.is_some()));
    }
    assert!(completed
        .artifacts
        .iter()
        .all(|artifact| Path::new(&artifact.path).is_file()));
    assert!(completed
        .navigation_targets
        .iter()
        .any(|target| target.kind == "workspace"));
    let log = completed.log_summary.join("\n");
    assert!(log.contains("Runner command prepared: local PDF conversion wrapper"));
    assert!(log.contains("Local PDF fixture wrapper completed"));
    assert!(!log.contains("DONE: sample.pdf -> sample.html"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn local_pdf_paddle_layout_hands_the_wrapper_markdown_to_translation() {
    let root = temp_root("local-pdf-paddle-handoff");
    let input = root.join("input");
    fs::create_dir_all(&input).unwrap();
    fs::write(input.join("Sample Book.pdf"), "%PDF fixture").unwrap();
    let repo_root = root.join("repo");
    fs::create_dir_all(repo_root.join("tools")).unwrap();
    fs::write(repo_root.join("AGENTS.md"), "fixture").unwrap();
    fs::write(
        repo_root.join("tools").join("create_local_book_project.py"),
        "fixture",
    )
    .unwrap();
    let wrapper_root = fake_wrapper_root(&root);
    let store = BookPipelineStore::for_test(&root);
    let job = queue_job(
        &store,
        local_pdf_source(&input),
        MODE_CONVERT_THEN_TRANSLATE.into(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();
    assert!(job
        .route
        .iter()
        .any(|item| item.route_kind == "remote_paddleocr"));

    let completed = run_job_with_handoff(
        &store,
        &CommandPipelineRunner::with_book_ocr_conversion_root(
            PaddleWrapperLayoutExecutor,
            wrapper_root,
        ),
        &LocalProjectHandoffRunner,
        &job.id,
        Some(&repo_root),
    )
    .unwrap();

    assert_eq!(completed.status, STATUS_READY);
    assert_eq!(completed.current_step, "Translation handoff ready");
    assert!(completed.last_error.is_none());
    let markdown = completed
        .artifacts
        .iter()
        .find(|artifact| artifact.kind == "markdown")
        .expect("the wrapper Markdown is registered as a markdown artifact");
    assert!(
        markdown.path.ends_with("Sample_Book.md"),
        "{}",
        markdown.path
    );
    let project_root = PathBuf::from(
        completed
            .children
            .iter()
            .find_map(|child| child.local_project_root.as_deref())
            .expect("registered local project root"),
    );
    assert_eq!(
        fs::read_to_string(project_root.join("source").join("source.md")).unwrap(),
        PADDLE_WRAPPER_MARKDOWN
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn local_pdf_runner_contract_records_wrapper_failure() {
    let root = temp_root("local-pdf-wrapper-failure");
    let input = root.join("input");
    fs::create_dir_all(&input).unwrap();
    fs::write(input.join("sample.pdf"), "%PDF fixture").unwrap();
    let wrapper_root = fake_wrapper_root(&root);
    let store = BookPipelineStore::for_test(&root);
    let job = queue_job(
        &store,
        local_pdf_source(&input),
        "conversion_only".into(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();

    let failed = run_job(
        &store,
        &CommandPipelineRunner::with_book_ocr_conversion_root(
            LocalPdfFailingExecutor,
            wrapper_root,
        ),
        &job.id,
    )
    .unwrap();

    assert_eq!(failed.status, STATUS_FAILED);
    assert_eq!(failed.current_step, "Failed");
    assert_eq!(failed.attempts, 1);
    assert_eq!(
        failed.last_error.as_deref(),
        Some("Local PDF fixture wrapper failed")
    );
    assert!(failed
        .log_summary
        .iter()
        .any(|line| line.contains("Runner failed: Local PDF fixture wrapper failed")));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn translate_only_markdown_source_creates_local_reading_project() {
    let root = temp_root("translate-only-markdown");
    let repo = root.join("repo");
    fs::create_dir_all(repo.join("tools")).unwrap();
    fs::write(repo.join("AGENTS.md"), "# fixture\n").unwrap();
    fs::write(
        repo.join("tools").join("create_local_book_project.py"),
        "# fixture\n",
    )
    .unwrap();
    let source_path = root.join("source.md");
    fs::write(&source_path, "# Source\n\nText to translate.\n").unwrap();
    let store = BookPipelineStore::for_test(&root);
    let job = queue_job(
        &store,
        markdown_source(&source_path),
        MODE_TRANSLATE_ONLY.into(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();

    let completed = run_job_with_handoff(
        &store,
        &SystemPipelineRunner,
        &LocalProjectHandoffRunner,
        &job.id,
        Some(&repo),
    )
    .unwrap();

    assert_eq!(completed.status, STATUS_READY);
    assert_eq!(completed.current_stage_id, "split");
    assert!(completed
        .route
        .iter()
        .any(|item| item.route_kind == "translation_ready" && item.can_run));
    let translation_source = completed
        .artifacts
        .iter()
        .find(|artifact| artifact.kind == "translation_source")
        .unwrap();
    assert_eq!(
        fs::read_to_string(&translation_source.path).unwrap(),
        "# Source\n\nText to translate.\n"
    );
    assert!(completed.children[0]
        .local_project_root
        .as_deref()
        .is_some_and(|path| Path::new(path).is_dir()));
    let manifest_path = Path::new(&translation_source.path)
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("metadata")
        .join("source_manifest.json");
    let manifest = fs::read_to_string(manifest_path).unwrap();
    assert!(manifest.contains("\"source_sha256\""));
    assert!(manifest.contains("\"extraction_status\": \"cleaned_markdown_ready\""));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn translate_only_rejects_runtime_staging_markdown_paths() {
    let root = temp_root("translate-only-staging-block");
    let staging = root.join(".state").join("staging").join("source.md");
    fs::create_dir_all(staging.parent().unwrap()).unwrap();
    fs::write(&staging, "# temporary\n").unwrap();

    let route = preview_route(
        &markdown_source(&staging),
        MODE_TRANSLATE_ONLY,
        BookPipelinePreviewConfig::default(),
    );

    assert!(!route[0].can_run);
    assert_eq!(
        route[0].blocked_reason.as_deref(),
        Some("OCR runtime staging paths are rejected by default.")
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn mineru_runner_command_records_artifacts() {
    let root = temp_root("mineru-wrapper-artifacts");
    let worker_root = fake_full_worker_root(&root);
    let store = BookPipelineStore::for_test(&root);
    let job = queue_job(
        &store,
        fake_mineru_zotero_source(),
        "conversion_only".into(),
        BookPipelinePreviewConfig {
            has_paddleocr_credentials: false,
            has_mineru_credentials: true,
            route_overrides: BTreeMap::new(),
        },
    )
    .unwrap();

    let completed = run_job(
        &store,
        &CommandPipelineRunner::with_book_ocr_conversion_root(MineruFixtureExecutor, worker_root),
        &job.id,
    )
    .unwrap();

    assert_eq!(completed.status, STATUS_COMPLETED);
    assert!(completed
        .route
        .iter()
        .any(|item| item.route_kind == "mineru" && item.can_run));
    assert!(completed
        .artifacts
        .iter()
        .any(|artifact| artifact.kind == "markdown" && artifact.sha256.is_some()));
    assert!(completed
        .log_summary
        .iter()
        .any(|line| line.contains("Runner command prepared: Zotero conversion worker")));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn external_adapter_route_normalizes_outputs() {
    let root = temp_root("external-adapter");
    fs::create_dir_all(&root).unwrap();
    let adapter = root.join("adapter.sh");
    fs::write(&adapter, "#!/bin/sh\n").unwrap();
    let input = root.join("input.pdf");
    fs::write(&input, "%PDF fixture").unwrap();
    let source = BookPipelineSource {
        kind: "external_adapter".into(),
        title: Some("External Adapter".into()),
        path: Some(display_path(&input)),
        selector: None,
        runner_behavior: None,
        adapter_command: Some(display_path(&adapter)),
        fake_zotero_items: None,
        route_overrides: BTreeMap::new(),
    };
    let store = BookPipelineStore::for_test(&root);
    let job = queue_job(
        &store,
        source,
        "conversion_only".into(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();

    let completed = run_job(
        &store,
        &CommandPipelineRunner::new(ExternalAdapterFixtureExecutor),
        &job.id,
    )
    .unwrap();

    assert_eq!(completed.status, STATUS_COMPLETED);
    assert!(completed
        .route
        .iter()
        .any(|item| item.route_kind == "external_adapter" && item.can_run));
    assert!(completed
        .artifacts
        .iter()
        .any(|artifact| artifact.kind == "markdown"));
    assert!(completed
        .artifacts
        .iter()
        .any(|artifact| artifact.kind == "html"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn zotero_collection_runs_direct_ocr_and_mineru_items_independently() {
    let root = temp_root("zotero-collection-mixed");
    let worker_root = fake_full_worker_root(&root);
    let store = BookPipelineStore::for_test(&root);
    let job = queue_job(
        &store,
        fake_collection_source(),
        "conversion_only".into(),
        BookPipelinePreviewConfig {
            has_paddleocr_credentials: true,
            has_mineru_credentials: true,
            route_overrides: BTreeMap::new(),
        },
    )
    .unwrap();

    let completed = run_job(
        &store,
        &CommandPipelineRunner::with_book_ocr_conversion_root(
            ZoteroBatchFixtureExecutor,
            worker_root,
        ),
        &job.id,
    )
    .unwrap();

    assert_eq!(completed.status, STATUS_PARTIAL);
    assert_eq!(
        completed
            .collection_items
            .iter()
            .filter(|item| item.status == STATUS_COMPLETED)
            .count(),
        3
    );
    assert!(completed.collection_items.iter().any(|item| {
        item.id == "DIRTY" && item.status == STATUS_BLOCKED && item.last_error.is_some()
    }));
    assert!(completed
        .collection_items
        .iter()
        .any(|item| item.id == "DONE" && item.status == "skipped"));
    assert!(completed.current_step.contains("completed=3"));
    assert!(completed.current_step.contains("blocked=1"));
    assert!(completed.current_step.contains("skipped=1"));
    assert!(completed.artifacts.iter().any(|artifact| {
        artifact.kind == "markdown" && artifact.zotero_key.as_deref() == Some("DIRECTMD")
    }));
    assert!(completed.artifacts.iter().any(|artifact| {
        artifact.kind == "markdown" && artifact.zotero_key.as_deref() == Some("SCANMD")
    }));
    assert!(completed.artifacts.iter().any(|artifact| {
        artifact.kind == "markdown" && artifact.zotero_key.as_deref() == Some("MINERUMD")
    }));
    assert!(completed.children.iter().all(|child| {
        stage_ref(child, "extract").unwrap().status != STATUS_COMPLETED
            || stage_ref(child, "index").unwrap().status == STATUS_COMPLETED
    }));
    let manifest = completed
        .artifacts
        .iter()
        .find(|artifact| artifact.kind == "collection_manifest")
        .expect("hashed collection summary manifest");
    assert!(manifest.validation.hash_matches);
    assert!(Path::new(&manifest.path).is_file());
    assert_eq!(
        completed
            .open_target
            .as_ref()
            .map(|target| target.action_label.as_str()),
        Some("Inspect partial results")
    );
    let selected = completed.open_target.as_ref().unwrap();
    let target = completed
        .navigation_targets
        .iter()
        .find(|target| target.target_id == selected.target_id)
        .unwrap();
    assert_eq!(
        target.artifact_id.as_deref(),
        Some(manifest.artifact_id.as_str())
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn zotero_collection_retry_targets_failed_items_only() {
    let root = temp_root("zotero-collection-retry");
    let worker_root = fake_zotero_worker_root(&root);
    let store = BookPipelineStore::for_test(&root);
    let source = BookPipelineSource {
        kind: "zotero_collection".into(),
        title: Some("Retry collection".into()),
        path: None,
        selector: Some("RETRY".into()),
        runner_behavior: None,
        adapter_command: None,
        fake_zotero_items: Some(vec![
            FakeZoteroItem {
                key: "OK".into(),
                title: "Already OK".into(),
                attachment_path: Some("zotero://attachment/OK".into()),
                has_text_layer: true,
                dirty_text_layer: false,
                scanned: false,
                already_converted: false,
                prefer_mineru: false,
            },
            FakeZoteroItem {
                key: "FAIL".into(),
                title: "Fails Once".into(),
                attachment_path: Some("zotero://attachment/FAIL".into()),
                has_text_layer: true,
                dirty_text_layer: false,
                scanned: false,
                already_converted: false,
                prefer_mineru: false,
            },
        ]),
        route_overrides: BTreeMap::new(),
    };
    let job = queue_job(
        &store,
        source,
        "conversion_only".into(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();
    let executor = RetryCollectionExecutor {
        fail_once: std::sync::Mutex::new(true),
    };

    let partial = run_job(
        &store,
        &CommandPipelineRunner::with_book_ocr_conversion_root(executor, worker_root.clone()),
        &job.id,
    )
    .unwrap();

    assert_eq!(partial.status, STATUS_PARTIAL);
    assert!(partial.collection_items.iter().any(|item| {
        item.id == "OK"
            && item.status == STATUS_COMPLETED
            && item
                .artifacts
                .iter()
                .any(|artifact| artifact.kind == "markdown")
    }));
    assert!(partial.collection_items.iter().any(|item| {
        item.id == "FAIL"
            && item.status == STATUS_FAILED
            && item
                .last_error
                .as_deref()
                .is_some_and(|error| error.contains("first attempt failed"))
    }));
    let executor = RetryCollectionExecutor {
        fail_once: std::sync::Mutex::new(false),
    };

    let completed = run_job(
        &store,
        &CommandPipelineRunner::with_book_ocr_conversion_root(executor, worker_root),
        &job.id,
    )
    .unwrap();

    assert_eq!(completed.status, STATUS_COMPLETED);
    assert!(completed
        .collection_items
        .iter()
        .any(|item| { item.id == "OK" && item.status == STATUS_COMPLETED && item.attempts == 1 }));
    assert!(completed.collection_items.iter().any(|item| {
        item.id == "FAIL" && item.status == STATUS_COMPLETED && item.attempts == 2
    }));
    assert!(completed.current_step.contains("failed=0"));
    assert!(completed
        .artifacts
        .iter()
        .any(|artifact| artifact.kind == "markdown" && artifact.path.contains("OK")));
    assert!(completed
        .artifacts
        .iter()
        .any(|artifact| artifact.kind == "markdown" && artifact.path.contains("FAIL")));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn zotero_discovery_parses_worker_dry_run_plan_sources() {
    let root = temp_root("zotero-discovery-plan");
    let worker_root = fake_zotero_worker_root(&root);
    let source = BookPipelineSource {
        kind: "zotero_filter".into(),
        title: Some("Book filter".into()),
        path: None,
        selector: Some("parent_item_type=book".into()),
        runner_behavior: None,
        adapter_command: None,
        fake_zotero_items: None,
        route_overrides: BTreeMap::new(),
    };

    let result =
        discover_zotero_sources_with_root(&ZoteroDiscoveryExecutor, &source, 5, &worker_root)
            .unwrap();

    assert!(result
        .sources
        .iter()
        .any(|source| source.kind == "zotero_filter"
            && source.selector.as_deref() == Some("parent_item_type=book")));
    let direct = result
        .sources
        .iter()
        .find(|source| source.selector.as_deref() == Some("DIRECT1"))
        .unwrap();
    assert_eq!(direct.kind, "zotero_attachment");
    assert_eq!(direct.title.as_deref(), Some("Born Digital Book"));
    let direct_item = &direct.fake_zotero_items.as_ref().unwrap()[0];
    assert!(direct_item.has_text_layer);
    assert!(!direct_item.scanned);

    let scanned = result
        .sources
        .iter()
        .find(|source| source.selector.as_deref() == Some("SCAN1"))
        .unwrap();
    let scanned_item = &scanned.fake_zotero_items.as_ref().unwrap()[0];
    assert!(!scanned_item.has_text_layer);
    assert!(scanned_item.scanned);
    assert!(result
        .log_summary
        .iter()
        .any(|line| line.contains("Runner command prepared: Zotero discovery dry-run")));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn zotero_query_filter_reads_the_same_prefixes_as_a_title_search_box() {
    // No prefix at all -- and no other recognised prefix -- means "no
    // search", the same way an empty selector means no filter today.
    assert_eq!(zotero_query_filter(Some("just typed text")), None);
    assert_eq!(zotero_query_filter(None), None);
    for prefix in ["query=", "query:", "q=", "q:", "title=", "title:"] {
        assert_eq!(
            zotero_query_filter(Some(&format!("{prefix}Geschäftsgeheimnisse"))),
            Some("Geschäftsgeheimnisse".to_string()),
            "prefix {prefix} was not recognised",
        );
    }
}

struct ZoteroQuerySelectorExecutor;

impl RunnerCommandExecutor for ZoteroQuerySelectorExecutor {
    fn execute(&self, command: &RunnerCommand) -> Result<RunnerCommandResult, String> {
        assert!(has_arg_pair(
            &command.args,
            "--query",
            "Geschäftsgeheimnisse"
        ));
        assert!(!command.args.iter().any(|arg| arg == "--parent-item-type"));
        Ok(RunnerCommandResult {
            stdout: String::new(),
            stderr: String::new(),
            log_summary: vec!["Zotero dry-run completed".into()],
        })
    }
}

#[test]
fn a_title_search_selector_reaches_the_discovery_command_as_query() {
    let root = temp_root("zotero-discovery-query");
    let worker_root = fake_zotero_worker_root(&root);
    let source = BookPipelineSource {
        kind: "zotero_filter".into(),
        title: Some("Title search".into()),
        path: None,
        selector: Some("query=Geschäftsgeheimnisse".into()),
        runner_behavior: None,
        adapter_command: None,
        fake_zotero_items: None,
        route_overrides: BTreeMap::new(),
    };

    discover_zotero_sources_with_root(&ZoteroQuerySelectorExecutor, &source, 5, &worker_root)
        .unwrap();
    let _ = fs::remove_dir_all(root);
}

#[test]
fn fake_zotero_discovery_items_do_not_execute_worker() {
    let source = BookPipelineSource {
        kind: "zotero_collection".into(),
        title: Some("Fake collection".into()),
        path: None,
        selector: Some("COLLECTION".into()),
        runner_behavior: None,
        adapter_command: None,
        fake_zotero_items: Some(vec![FakeZoteroItem {
            key: "FAKE1".into(),
            title: "Fake attachment".into(),
            attachment_path: Some("zotero://fake/FAKE1.pdf".into()),
            has_text_layer: true,
            dirty_text_layer: false,
            scanned: false,
            already_converted: false,
            prefer_mineru: false,
        }]),
        route_overrides: BTreeMap::new(),
    };

    let result = discover_zotero_sources(&PanicExecutor, &source, 20).unwrap();

    assert_eq!(result.sources.len(), 1);
    assert_eq!(result.sources[0].kind, "zotero_collection");
    assert_eq!(
        result.sources[0].fake_zotero_items.as_ref().unwrap()[0].key,
        "FAKE1"
    );
}

#[test]
fn zotero_discovery_failure_is_redacted() {
    let root = temp_root("zotero-discovery-redacted");
    let worker_root = fake_zotero_worker_root(&root);
    let source = BookPipelineSource {
        kind: "zotero_attachment".into(),
        title: Some("Single attachment".into()),
        path: None,
        selector: Some("ABC123".into()),
        runner_behavior: None,
        adapter_command: None,
        fake_zotero_items: None,
        route_overrides: BTreeMap::new(),
    };

    let error = discover_zotero_sources_with_root(
        &ZoteroDiscoverySecretFailingExecutor,
        &source,
        1,
        &worker_root,
    )
    .unwrap_err();

    assert_eq!(
        error,
        "Sensitive credential or signed-request details were redacted."
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn zotero_worker_route_carries_the_handoff_row_like_every_other_source_kind() {
    let root = temp_root("zotero-route-handoff");
    let worker_root = fake_zotero_worker_root(&root);
    let source = BookPipelineSource {
        kind: "zotero_filter".into(),
        title: Some("Handoff parity".into()),
        path: None,
        selector: Some("parent_item_type=book".into()),
        runner_behavior: None,
        adapter_command: None,
        fake_zotero_items: None,
        route_overrides: BTreeMap::new(),
    };
    let config = || BookPipelinePreviewConfig {
        has_paddleocr_credentials: true,
        has_mineru_credentials: true,
        route_overrides: BTreeMap::new(),
    };

    // A conversion-only run ends at conversion, so no handoff row.
    let conversion_only = preview_zotero_route_from_worker(
        &ZoteroRoutePreviewExecutor,
        &source,
        "conversion_only",
        config(),
        20,
        &worker_root,
    )
    .expect("worker preview should resolve");
    assert!(
        !conversion_only
            .iter()
            .any(|item| item.route_kind == "translation_handoff"),
        "conversion_only must not promise a handoff"
    );

    // A handoff mode must carry it, exactly as preview_route does for every
    // other source kind -- the wizard's preflight table would otherwise
    // under-report the work for live Zotero sources only.
    for mode in [MODE_CONVERT_THEN_TRANSLATE, MODE_TRANSLATE_ONLY] {
        let route = preview_zotero_route_from_worker(
            &ZoteroRoutePreviewExecutor,
            &source,
            mode,
            config(),
            20,
            &worker_root,
        )
        .expect("worker preview should resolve");
        assert!(
            route
                .iter()
                .any(|item| item.route_kind == "translation_handoff"),
            "{mode} must carry the handoff row"
        );
        assert_eq!(
            route.last().map(|item| item.route_kind.as_str()),
            Some("translation_handoff"),
            "{mode} must append the handoff row last, as preview_route does"
        );
    }

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn zotero_route_preview_uses_worker_dry_run_policy() {
    let root = temp_root("zotero-route-preview");
    let worker_root = fake_zotero_worker_root(&root);
    let source = BookPipelineSource {
        kind: "zotero_filter".into(),
        title: Some("Preview queue".into()),
        path: None,
        selector: Some("parent_item_type=book".into()),
        runner_behavior: None,
        adapter_command: None,
        fake_zotero_items: None,
        route_overrides: BTreeMap::new(),
    };

    let route = preview_zotero_route_from_worker(
        &ZoteroRoutePreviewExecutor,
        &source,
        "conversion_only",
        BookPipelinePreviewConfig {
            has_paddleocr_credentials: false,
            has_mineru_credentials: true,
            route_overrides: BTreeMap::new(),
        },
        20,
        &worker_root,
    )
    .unwrap();

    assert!(route
        .iter()
        .any(|item| item.id == "DIRECT" && item.route_kind == "direct_text" && item.can_run));
    assert!(route.iter().any(|item| {
        item.id == "SCAN"
            && item.route_kind == "missing_credentials"
            && !item.can_run
            && item
                .blocked_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("PaddleOCR"))
    }));
    assert!(route
        .iter()
        .any(|item| item.id == "MINERU" && item.route_kind == "mineru" && item.can_run));
    assert!(route.iter().any(|item| {
        item.id == "DIRTY"
            && item.route_kind == "blocked_dirty_text_layer"
            && !item.can_run
            && item.blocked_reason.is_some()
    }));
    assert!(route.iter().any(|item| {
        item.id == "DONE" && item.route_kind == "already_converted" && !item.can_run
    }));

    let route_with_remote_ocr = preview_zotero_route_from_worker(
        &ZoteroRoutePreviewExecutor,
        &source,
        "conversion_only",
        BookPipelinePreviewConfig {
            has_paddleocr_credentials: true,
            has_mineru_credentials: true,
            route_overrides: BTreeMap::new(),
        },
        20,
        &worker_root,
    )
    .unwrap();
    assert!(route_with_remote_ocr.iter().any(|item| {
        item.id == "SCAN" && item.route_kind == "remote_paddleocr" && item.can_run
    }));
    assert!(route_with_remote_ocr
        .iter()
        .any(|item| item.id == "DIRECT" && item.can_run));
    let _ = fs::remove_dir_all(root);
}

fn zotero_query_filter_source() -> BookPipelineSource {
    BookPipelineSource {
        kind: "zotero_filter".into(),
        title: Some("Title search".into()),
        path: None,
        selector: Some("query=Der wirtschaftliche Wert".into()),
        runner_behavior: None,
        adapter_command: None,
        fake_zotero_items: None,
        route_overrides: BTreeMap::new(),
    }
}

#[test]
fn zotero_filter_queue_discovers_real_children_from_worker() {
    let root = temp_root("zotero-filter-queue-discovery");
    let worker_root = fake_zotero_worker_root(&root);
    let store = BookPipelineStore::for_test(&root);

    let job = queue_standard_job_for_root(
        &store,
        &ZoteroRoutePreviewExecutor,
        zotero_query_filter_source(),
        "conversion_only".into(),
        fast_translation_intent(),
        BookPipelinePreviewConfig {
            has_paddleocr_credentials: false,
            has_mineru_credentials: true,
            route_overrides: BTreeMap::from([("SCAN".into(), "direct".into())]),
        },
        &worker_root,
    )
    .unwrap();

    assert_eq!(job.kind, "collection");
    assert_eq!(job.status, STATUS_READY);
    let route_ids: Vec<&str> = job.route.iter().map(|item| item.id.as_str()).collect();
    assert_eq!(route_ids, ["DIRECT", "SCAN", "MINERU", "DIRTY", "DONE"]);
    assert!(job.route.iter().all(|item| !item.id.contains("query=")));
    let scan = job.route.iter().find(|item| item.id == "SCAN").unwrap();
    assert_eq!(scan.route_kind, "direct_text");
    assert!(scan.can_run);
    assert_eq!(scan.route_override.as_deref(), Some("direct"));

    assert_eq!(job.children.len(), 5);
    for child in &job.children {
        assert_eq!(child.source.kind, "zotero_attachment");
        let selector = child.source.selector.as_deref().unwrap();
        assert!(route_ids.contains(&selector));
    }
    let _ = fs::remove_dir_all(root);
}

struct ZoteroEmptyDiscoveryExecutor;

impl RunnerCommandExecutor for ZoteroEmptyDiscoveryExecutor {
    fn execute(&self, command: &RunnerCommand) -> Result<RunnerCommandResult, String> {
        assert_eq!(command.label, "Zotero discovery dry-run");
        Ok(RunnerCommandResult {
            stdout: String::new(),
            stderr: String::new(),
            log_summary: vec!["Zotero dry-run found nothing".into()],
        })
    }
}

#[test]
fn zotero_filter_queue_with_no_matches_blocks_without_demo_children() {
    let root = temp_root("zotero-filter-queue-no-matches");
    let worker_root = fake_zotero_worker_root(&root);
    let store = BookPipelineStore::for_test(&root);

    let job = queue_standard_job_for_root(
        &store,
        &ZoteroEmptyDiscoveryExecutor,
        zotero_query_filter_source(),
        "conversion_only".into(),
        fast_translation_intent(),
        BookPipelinePreviewConfig::default(),
        &worker_root,
    )
    .unwrap();

    assert_eq!(job.status, STATUS_BLOCKED);
    assert_eq!(job.route.len(), 1);
    assert_eq!(job.route[0].route_kind, "blocked_no_attachment");
    assert!(!job.route[0].can_run);
    assert!(job.route[0]
        .blocked_reason
        .as_deref()
        .is_some_and(|reason| reason.contains("No matching Zotero attachment")));
    assert!(job
        .children
        .iter()
        .all(|child| !child.id.contains("-DIRECT") && child.status != STATUS_READY));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn zotero_conversion_records_markdown_artifact_and_upload_key() {
    let root = temp_root("zotero-conversion-success");
    let worker_root = fake_zotero_worker_root(&root);
    let store = BookPipelineStore::for_test(&root);
    let job = queue_job(
        &store,
        fake_direct_zotero_source(),
        "conversion_only".into(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();
    assert_eq!(job.status, STATUS_READY);

    let completed = run_job(
        &store,
        &CommandPipelineRunner::with_book_ocr_conversion_root(
            ZoteroConversionExecutor,
            worker_root,
        ),
        &job.id,
    )
    .unwrap();

    assert_eq!(completed.status, STATUS_COMPLETED);
    assert_eq!(completed.current_step, "Completed");
    let markdown = completed
        .artifacts
        .iter()
        .find(|artifact| artifact.kind == "markdown")
        .unwrap();
    assert!(markdown.sha256.is_some());
    assert_eq!(markdown.zotero_key.as_deref(), Some("MDKEY123"));
    assert_eq!(
        markdown.source_refs.pdf_attachment_key.as_deref(),
        Some("DIRECT")
    );
    assert_eq!(
        markdown.source_refs.markdown_attachment_key.as_deref(),
        Some("MDKEY123")
    );
    assert!(completed
        .log_summary
        .iter()
        .any(|line| line.contains("Zotero Markdown attachment recorded: MDKEY123")));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn zotero_extraction_runs_item_scoped_index_before_completion() {
    let root = temp_root("zotero-extract-index-success");
    let worker_root = fake_zotero_worker_root(&root);
    let store = BookPipelineStore::for_test(&root);
    let job = queue_job(
        &store,
        fake_direct_zotero_source(),
        "conversion_only".into(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();
    let runner = CommandPipelineRunner::with_book_ocr_conversion_root(
        ZoteroExtractIndexExecutor::succeeding(),
        worker_root,
    );

    let completed = run_job(&store, &runner, &job.id).unwrap();

    assert_eq!(child_stage_status(&completed, "extract"), STATUS_COMPLETED);
    assert_eq!(child_stage_status(&completed, "index"), STATUS_COMPLETED);
    assert_eq!(completed.status, STATUS_COMPLETED);
    assert_eq!(
        runner.executor.command_labels(),
        vec![
            "Zotero conversion worker".to_string(),
            "Zotero item index profile".to_string(),
            "Zotero item-scoped full-text index".to_string(),
        ]
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn zotero_translation_handoff_waits_for_item_scoped_index() {
    let root = temp_root("zotero-extract-index-handoff");
    let worker_root = fake_zotero_worker_root(&root);
    let repo_root = root.join("repo");
    let store = BookPipelineStore::for_test(&root);
    let job = queue_job(
        &store,
        fake_direct_zotero_source(),
        MODE_CONVERT_THEN_TRANSLATE.into(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();
    let runner = CommandPipelineRunner::with_book_ocr_conversion_root(
        ZoteroExtractIndexExecutor::succeeding(),
        worker_root,
    );

    let handed_off = run_job_with_handoff(
        &store,
        &runner,
        &FakeTranslationHandoffRunner,
        &job.id,
        Some(&repo_root),
    )
    .unwrap();

    assert_eq!(child_stage_status(&handed_off, "extract"), STATUS_COMPLETED);
    assert_eq!(child_stage_status(&handed_off, "index"), STATUS_COMPLETED);
    assert_eq!(child_stage_status(&handed_off, "handoff"), STATUS_COMPLETED);
    assert_eq!(handed_off.current_stage_id, "split");
    assert!(handed_off
        .artifacts
        .iter()
        .any(|artifact| artifact.kind == "translation_source"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn item_scoped_index_persists_safe_evidence_without_chunk_text() {
    let root = temp_root("zotero-index-evidence");
    let worker_root = fake_zotero_worker_root(&root);
    let store = BookPipelineStore::for_test(&root);
    let job = queue_job(
        &store,
        fake_direct_zotero_source(),
        "conversion_only".into(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();
    let runner = CommandPipelineRunner::with_book_ocr_conversion_root(
        ZoteroExtractIndexExecutor::succeeding(),
        worker_root,
    );

    let completed = run_job(&store, &runner, &job.id).unwrap();

    let index_stage = completed.children[0]
        .stages
        .iter()
        .find(|stage| stage.stage_id == "index")
        .unwrap();
    let markdown = completed.children[0]
        .artifacts
        .iter()
        .find(|artifact| artifact.kind == "markdown")
        .unwrap();
    assert_eq!(
        index_stage.input_hashes.get("markdownArtifactId"),
        Some(&markdown.artifact_id)
    );
    let persisted = serde_json::to_value(index_stage).unwrap();
    assert_eq!(persisted["indexEvidence"]["parentItemKey"], "PARENT123");
    assert_eq!(persisted["indexEvidence"]["chunkCount"], 1);
    assert_eq!(
        persisted["indexEvidence"]["indexContractVersion"],
        ITEM_INDEX_CONTRACT_VERSION
    );
    assert_eq!(
        persisted["indexEvidence"]["embeddingProfileId"],
        "fixture-embedding:3"
    );
    assert!(!persisted.to_string().contains("Direct Markdown"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn zotero_extraction_without_markdown_attachment_key_does_not_start_index() {
    let root = temp_root("zotero-index-requires-upload-key");
    let worker_root = fake_zotero_worker_root(&root);
    let store = BookPipelineStore::for_test(&root);
    let job = queue_job(
        &store,
        fake_direct_zotero_source(),
        "conversion_only".into(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();
    let runner = CommandPipelineRunner::with_book_ocr_conversion_root(
        ZoteroExtractIndexExecutor::missing_markdown_attachment_key(),
        worker_root,
    );

    let failed = run_job(&store, &runner, &job.id).unwrap();

    assert_eq!(child_stage_status(&failed, "extract"), STATUS_FAILED);
    assert_eq!(child_stage_status(&failed, "index"), STATUS_PENDING);
    assert_eq!(
        runner.executor.command_labels(),
        vec!["Zotero conversion worker".to_string()]
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn completed_item_index_requires_matching_persisted_evidence() {
    let root = temp_root("zotero-index-evidence-validation");
    let worker_root = fake_zotero_worker_root(&root);
    let store = BookPipelineStore::for_test(&root);
    let job = queue_job(
        &store,
        fake_direct_zotero_source(),
        "conversion_only".into(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();
    let runner = CommandPipelineRunner::with_book_ocr_conversion_root(
        ZoteroExtractIndexExecutor::succeeding(),
        worker_root,
    );
    run_job(&store, &runner, &job.id).unwrap();
    let mut state = store.load().unwrap();
    stage_mut(&mut state.jobs[0].children[0], "index")
        .unwrap()
        .index_evidence = None;

    let error = validate_state(&state).unwrap_err();

    assert!(error.contains("completed index evidence"));

    let mut state = store.load().unwrap();
    stage_mut(&mut state.jobs[0].children[0], "index")
        .unwrap()
        .index_evidence
        .as_mut()
        .unwrap()
        .parent_item_key = "WRONGPARENT".into();
    let error = validate_state(&state).unwrap_err();
    assert!(error.contains("mismatched completed index evidence"));

    let mut state = store.load().unwrap();
    stage_mut(&mut state.jobs[0].children[0], "index")
        .unwrap()
        .input_hashes
        .insert("markdownArtifactId".into(), "artifact-wrong".into());
    let error = validate_state(&state).unwrap_err();
    assert!(error.contains("mismatched completed index evidence"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn missing_markdown_is_persisted_as_an_index_failure() {
    let root = temp_root("zotero-index-missing-markdown");
    let store = BookPipelineStore::for_test(&root);
    let job = queue_job(
        &store,
        fake_direct_zotero_source(),
        "conversion_only".into(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();

    let failed = run_job(&store, &MissingMarkdownRunner, &job.id).unwrap();

    assert_eq!(child_stage_status(&failed, "extract"), STATUS_COMPLETED);
    assert_eq!(child_stage_status(&failed, "index"), STATUS_FAILED);
    assert_eq!(stage_ref(&failed.children[0], "index").unwrap().attempt, 1);
    assert!(failed
        .last_error
        .as_deref()
        .is_some_and(|error| error.contains("no Markdown artifact")));
    let recovered = store.load().unwrap();
    assert_eq!(
        child_stage_status(&recovered.jobs[0], "index"),
        STATUS_FAILED
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn item_scoped_index_retry_does_not_rerun_extraction() {
    let root = temp_root("zotero-index-retry");
    let worker_root = fake_zotero_worker_root(&root);
    let store = BookPipelineStore::for_test(&root);
    let job = queue_job(
        &store,
        fake_direct_zotero_source(),
        "conversion_only".into(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();
    let runner = CommandPipelineRunner::with_book_ocr_conversion_root(
        ZoteroExtractIndexExecutor::failing_index_once(),
        worker_root,
    );

    let failed = run_job(&store, &runner, &job.id).unwrap();

    assert_eq!(child_stage_status(&failed, "extract"), STATUS_COMPLETED);
    assert_eq!(child_stage_status(&failed, "index"), STATUS_FAILED);
    let failed_index = stage_ref(&failed.children[0], "index").unwrap();
    let markdown = failed.children[0]
        .artifacts
        .iter()
        .find(|artifact| artifact.kind == "markdown")
        .unwrap();
    assert_eq!(
        failed_index.input_hashes.get("markdownArtifactId"),
        Some(&markdown.artifact_id)
    );
    assert_eq!(
        failed_index.input_hashes.get("markdownSha256"),
        markdown.sha256.as_ref()
    );
    assert_eq!(
        failed_index.input_hashes.get("embeddingProfileId"),
        Some(&"fixture-embedding:3".to_string())
    );
    assert!(failed.children[0]
        .artifacts
        .iter()
        .any(|artifact| artifact.kind == "markdown" && artifact.validation.hash_matches));

    let completed = run_job(&store, &runner, &job.id).unwrap();

    assert_eq!(child_stage_status(&completed, "extract"), STATUS_COMPLETED);
    assert_eq!(child_stage_status(&completed, "index"), STATUS_COMPLETED);
    assert_eq!(completed.status, STATUS_COMPLETED);
    assert_eq!(
        runner.executor.command_labels(),
        vec![
            "Zotero conversion worker".to_string(),
            "Zotero item index profile".to_string(),
            "Zotero item-scoped full-text index".to_string(),
            "Zotero item-scoped full-text index".to_string(),
        ]
    );
    assert_eq!(
        stage_ref(&completed.children[0], "extract")
            .unwrap()
            .attempt,
        1
    );
    assert_eq!(
        stage_ref(&completed.children[0], "index").unwrap().attempt,
        2
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn zotero_preview_reconciles_completed_and_changed_fingerprints() {
    let root = temp_root("zotero-fingerprint-preview");
    let worker_root = fake_zotero_worker_root(&root);
    let source = BookPipelineSource {
        kind: "zotero_filter".into(),
        title: Some("Fingerprint queue".into()),
        path: None,
        selector: Some("parent_item_type=book".into()),
        runner_behavior: None,
        adapter_command: None,
        fake_zotero_items: None,
        route_overrides: BTreeMap::new(),
    };

    let route = preview_zotero_route_from_worker(
        &ZoteroFingerprintPreviewExecutor,
        &source,
        "conversion_only",
        BookPipelinePreviewConfig {
            has_paddleocr_credentials: true,
            has_mineru_credentials: true,
            route_overrides: BTreeMap::new(),
        },
        20,
        &worker_root,
    )
    .unwrap();

    let current = route.iter().find(|item| item.id == "CURRENT").unwrap();
    assert_eq!(current.route_kind, "already_converted");
    assert!(!current.can_run);
    assert!(current.source_ref.contains("/tmp/current.md"));
    assert!(current.source_ref.contains("source_md5=aaa111"));

    let missing_upload = route.iter().find(|item| item.id == "MISSING").unwrap();
    assert_eq!(missing_upload.route_kind, "direct_text");
    assert!(missing_upload.can_run);
    assert!(missing_upload.source_ref.contains("source_md5=aaa111"));

    let changed = route.iter().find(|item| item.id == "CHANGED").unwrap();
    assert_eq!(changed.route_kind, "direct_text");
    assert!(changed.can_run);
    assert!(changed.source_ref.contains("source_md5=bbb222"));

    let blocked = route.iter().find(|item| item.id == "DIRTY").unwrap();
    assert_eq!(blocked.route_kind, "blocked_dirty_text_layer");
    assert!(!blocked.can_run);
    assert!(blocked.source_ref.contains("source_md5=ccc333"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn zotero_conversion_retry_preserves_failure_diagnosis_in_logs() {
    let root = temp_root("zotero-conversion-retry");
    let worker_root = fake_zotero_worker_root(&root);
    let store = BookPipelineStore::for_test(&root);
    let job = queue_job(
        &store,
        fake_direct_zotero_source(),
        "conversion_only".into(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();

    let failed = run_job(
        &store,
        &CommandPipelineRunner::with_book_ocr_conversion_root(
            ZoteroConversionFailingExecutor,
            worker_root.clone(),
        ),
        &job.id,
    )
    .unwrap();
    assert_eq!(failed.status, STATUS_FAILED);
    assert_eq!(
        failed.last_error.as_deref(),
        Some("Zotero conversion fixture failed: diagnosis preserved")
    );

    let completed = run_job(
        &store,
        &CommandPipelineRunner::with_book_ocr_conversion_root(
            ZoteroConversionExecutor,
            worker_root,
        ),
        &job.id,
    )
    .unwrap();
    assert_eq!(completed.status, STATUS_COMPLETED);
    assert!(completed.log_summary.iter().any(|line| {
        line.contains("Runner failed: Zotero conversion fixture failed: diagnosis preserved")
    }));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn zotero_mixed_blocked_route_keeps_runnable_items_available() {
    let root = temp_root("zotero-mixed-blocked");
    let store = BookPipelineStore::for_test(&root);
    let source = BookPipelineSource {
        kind: "zotero_filter".into(),
        title: Some("Mixed queue".into()),
        path: None,
        selector: Some("parent_item_type=book".into()),
        runner_behavior: None,
        adapter_command: None,
        fake_zotero_items: Some(vec![
            FakeZoteroItem {
                key: "DIRECT".into(),
                title: "Direct Text".into(),
                attachment_path: Some("zotero://attachment/DIRECT".into()),
                has_text_layer: true,
                dirty_text_layer: false,
                scanned: false,
                already_converted: false,
                prefer_mineru: false,
            },
            FakeZoteroItem {
                key: "SCAN".into(),
                title: "Scanned PDF".into(),
                attachment_path: Some("zotero://attachment/SCAN".into()),
                has_text_layer: false,
                dirty_text_layer: false,
                scanned: true,
                already_converted: false,
                prefer_mineru: false,
            },
        ]),
        route_overrides: BTreeMap::new(),
    };
    let job = queue_job(
        &store,
        source,
        "conversion_only".into(),
        BookPipelinePreviewConfig {
            has_paddleocr_credentials: false,
            has_mineru_credentials: false,
            route_overrides: BTreeMap::new(),
        },
    )
    .unwrap();
    assert_eq!(job.status, STATUS_READY);
    assert!(job
        .route
        .iter()
        .any(|item| item.id == "DIRECT" && item.route_kind == "direct_text" && item.can_run));
    assert!(job.route.iter().any(|item| {
        item.id == "SCAN" && item.route_kind == "missing_credentials" && !item.can_run
    }));
    let _ = fs::remove_dir_all(root);
}

fn override_route_source(overrides: BTreeMap<String, String>) -> BookPipelineSource {
    BookPipelineSource {
        kind: "zotero_filter".into(),
        title: Some("Override queue".into()),
        path: None,
        selector: Some("parent_item_type=book".into()),
        runner_behavior: None,
        adapter_command: None,
        fake_zotero_items: Some(vec![
            FakeZoteroItem {
                key: "DIRECT".into(),
                title: "Direct Text".into(),
                attachment_path: Some("zotero://attachment/DIRECT".into()),
                has_text_layer: true,
                dirty_text_layer: false,
                scanned: false,
                already_converted: false,
                prefer_mineru: false,
            },
            FakeZoteroItem {
                key: "SCAN".into(),
                title: "Scanned PDF".into(),
                attachment_path: Some("zotero://attachment/SCAN".into()),
                has_text_layer: false,
                dirty_text_layer: false,
                scanned: true,
                already_converted: false,
                prefer_mineru: false,
            },
        ]),
        route_overrides: overrides,
    }
}

#[test]
fn route_override_forces_mineru_over_automatic_direct_text() {
    let root = temp_root("route-override-mineru");
    let store = BookPipelineStore::for_test(&root);
    let job = queue_job(
        &store,
        override_route_source(BTreeMap::new()),
        "conversion_only".into(),
        BookPipelinePreviewConfig {
            has_paddleocr_credentials: true,
            has_mineru_credentials: true,
            route_overrides: BTreeMap::from([("DIRECT".into(), "mineru".into())]),
        },
    )
    .unwrap();

    let direct = job.route.iter().find(|item| item.id == "DIRECT").unwrap();
    assert_eq!(direct.route_kind, "mineru");
    assert_eq!(direct.route_override.as_deref(), Some("mineru"));
    assert!(direct.can_run);
    // The untouched item keeps its automatic decision and records no override.
    let scan = job.route.iter().find(|item| item.id == "SCAN").unwrap();
    assert!(scan.route_override.is_none());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn route_override_keep_marks_item_already_converted_and_not_runnable() {
    let root = temp_root("route-override-keep");
    let store = BookPipelineStore::for_test(&root);
    let job = queue_job(
        &store,
        override_route_source(BTreeMap::new()),
        "conversion_only".into(),
        BookPipelinePreviewConfig {
            has_paddleocr_credentials: true,
            has_mineru_credentials: true,
            route_overrides: BTreeMap::from([("SCAN".into(), "keep".into())]),
        },
    )
    .unwrap();

    let scan = job.route.iter().find(|item| item.id == "SCAN").unwrap();
    assert_eq!(scan.route_kind, "already_converted");
    assert!(!scan.can_run);
    assert_eq!(scan.route_override.as_deref(), Some("keep"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn route_override_cannot_bypass_missing_credentials() {
    let root = temp_root("route-override-no-bypass");
    let store = BookPipelineStore::for_test(&root);
    let job = queue_job(
        &store,
        override_route_source(BTreeMap::new()),
        "conversion_only".into(),
        BookPipelinePreviewConfig {
            has_paddleocr_credentials: false,
            has_mineru_credentials: false,
            route_overrides: BTreeMap::from([("DIRECT".into(), "mineru".into())]),
        },
    )
    .unwrap();

    // Forcing an engine whose credentials are absent must hold the item, not
    // hand it to a runner that cannot possibly succeed.
    let direct = job.route.iter().find(|item| item.id == "DIRECT").unwrap();
    assert_eq!(direct.route_kind, "missing_credentials");
    assert!(!direct.can_run);
    assert!(direct.blocked_reason.is_some());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn route_override_is_persisted_on_the_queued_source() {
    let root = temp_root("route-override-persisted");
    let store = BookPipelineStore::for_test(&root);
    let job = queue_job(
        &store,
        override_route_source(BTreeMap::new()),
        "conversion_only".into(),
        BookPipelinePreviewConfig {
            has_paddleocr_credentials: true,
            has_mineru_credentials: true,
            route_overrides: BTreeMap::from([("DIRECT".into(), "mineru".into())]),
        },
    )
    .unwrap();

    // Durable Zotero jobs re-derive their route at run time; the override has
    // to live on the stored source or it would be silently reverted there.
    assert_eq!(
        job.source.route_overrides.get("DIRECT").map(String::as_str),
        Some("mineru")
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn runner_failure_records_redacted_error_for_retry() {
    let root = temp_root("secret-failure");
    let store = BookPipelineStore::for_test(&root);
    let job = queue_job(
        &store,
        fake_source(None),
        "conversion_only".into(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();

    let failed = run_job(
        &store,
        &CommandPipelineRunner::new(SecretFailingExecutor),
        &job.id,
    )
    .unwrap();

    assert_eq!(failed.status, STATUS_FAILED);
    assert_eq!(failed.current_step, "Failed");
    assert_eq!(failed.attempts, 1);
    assert_eq!(
        failed.last_error.as_deref(),
        Some("Sensitive credential or signed-request details were redacted.")
    );
    let log = failed.log_summary.join("\n");
    assert!(log.contains("Sensitive credential"));
    assert!(!log.contains("supersecret"));
    assert!(!log.contains("abc"));
    assert!(!log.to_ascii_lowercase().contains("bearer"));
    let extract = failed.children[0]
        .stages
        .iter()
        .find(|stage| stage.stage_id == "extract")
        .unwrap();
    assert_eq!(
        extract.safe_error.as_ref().map(|error| error.code.as_str()),
        Some("missing_credentials")
    );
    let timestamp = extract.safe_error.as_ref().unwrap().timestamp.clone();
    let recovered = store.load().unwrap();
    let recovered_error = recovered
        .jobs
        .iter()
        .find(|stored| stored.id == job.id)
        .unwrap()
        .children[0]
        .stages
        .iter()
        .find(|stage| stage.stage_id == "extract")
        .and_then(|stage| stage.safe_error.as_ref())
        .unwrap();
    assert_eq!(recovered_error.timestamp, timestamp);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn runner_success_redacts_secret_streams_and_preserves_artifacts() {
    let root = temp_root("secret-success");
    let store = BookPipelineStore::for_test(&root);
    let job = queue_job(
        &store,
        fake_source(None),
        "conversion_only".into(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();

    let completed = run_job(
        &store,
        &CommandPipelineRunner::new(SecretLoggingExecutor),
        &job.id,
    )
    .unwrap();

    assert_eq!(completed.status, STATUS_COMPLETED);
    assert!(completed
        .artifacts
        .iter()
        .any(|artifact| artifact.kind == "markdown" && artifact.sha256.is_some()));
    let log = completed.log_summary.join("\n");
    assert!(log.contains("Runner command prepared: fake Book Pipeline runner"));
    assert!(log.contains("Sensitive credential"));
    assert!(!log.contains("supersecret"));
    assert!(!log.contains("abc"));
    assert!(!log.to_ascii_lowercase().contains("bearer"));
    // Merely naming .env, with no assignment or value attached, isn't a
    // leak and should stay legible — unlike the ZOTERO_API_KEY=... line
    // above it, which is a real leak and gets redacted.
    assert!(log.contains(".env content was not read"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn local_pdf_folder_runner_registers_artifact_checksums() {
    let root = temp_root("local-pdf");
    let input = root.join("input");
    fs::create_dir_all(&input).unwrap();
    fs::write(input.join("sample.pdf"), "%PDF fixture").unwrap();
    let store = BookPipelineStore::for_test(&root);
    let source = BookPipelineSource {
        kind: "local_pdf_folder".into(),
        title: Some("PDF folder".into()),
        path: Some(display_path(&input)),
        selector: None,
        runner_behavior: None,
        adapter_command: None,
        fake_zotero_items: None,
        route_overrides: BTreeMap::new(),
    };

    let job = queue_job(
        &store,
        source,
        "conversion_only".into(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();
    assert_eq!(job.route.len(), 1);
    assert_eq!(job.route[0].route_kind, "remote_paddleocr");

    let completed = run_job(&store, &ArtifactFixtureRunner, &job.id).unwrap();
    assert_eq!(completed.status, STATUS_COMPLETED);
    for kind in ["markdown", "html", "epub"] {
        assert!(completed
            .artifacts
            .iter()
            .any(|artifact| artifact.kind == kind && artifact.sha256.is_some()));
    }
    assert!(completed.output_dir.is_some());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn markdown_artifact_handoff_creates_translation_ready_project() {
    let root = temp_root("handoff-success");
    let repo_root = root.join("repo");
    fs::create_dir_all(repo_root.join("tools")).unwrap();
    fs::write(repo_root.join("AGENTS.md"), "fixture").unwrap();
    fs::write(
        repo_root.join("tools").join("create_local_book_project.py"),
        "fixture",
    )
    .unwrap();
    let store = BookPipelineStore::for_test(&root);
    let job = queue_job(
        &store,
        fake_source(None),
        "conversion_only".into(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();
    let completed = run_job(&store, &ArtifactFixtureRunner, &job.id).unwrap();
    assert_eq!(completed.status, STATUS_COMPLETED);

    let handed_off = handoff_job_markdown(&store, &job.id, None, &repo_root).unwrap();

    assert_eq!(handed_off.status, STATUS_READY);
    assert_eq!(handed_off.current_stage_id, "split");
    assert_eq!(handed_off.current_step, "Translation handoff ready");
    let project_root = PathBuf::from(
        handed_off.children[0]
            .local_project_root
            .as_deref()
            .expect("registered local project root"),
    );
    assert!(project_root.join("source").join("source.md").is_file());
    assert!(project_root.join("chapters").join("src").is_dir());
    let source_artifact = handed_off
        .artifacts
        .iter()
        .find(|artifact| artifact.kind == "translation_source")
        .unwrap();
    assert!(source_artifact.sha256.is_some());
    let manifest =
        fs::read_to_string(project_root.join("metadata").join("source_manifest.json")).unwrap();
    assert!(manifest.contains("cleaned_markdown_ready"));
    assert_eq!(
        fs::read_to_string(project_root.join("source").join("source.md")).unwrap(),
        fs::read_to_string(project_root.join("source").join("original.md")).unwrap()
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn markdown_artifact_handoff_rejects_missing_extraction_prerequisite() {
    let root = temp_root("handoff-failure");
    let repo_root = root.join("repo");
    fs::create_dir_all(repo_root.join("tools")).unwrap();
    fs::write(repo_root.join("AGENTS.md"), "fixture").unwrap();
    fs::write(
        repo_root.join("tools").join("create_local_book_project.py"),
        "fixture",
    )
    .unwrap();
    let store = BookPipelineStore::for_test(&root);
    let job = queue_job(
        &store,
        fake_source(None),
        "conversion_only".into(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();

    let error = handoff_job_markdown(&store, &job.id, None, &repo_root).unwrap_err();

    assert!(error.contains("No completed extraction"));
    let recovered = store.load().unwrap();
    let stored = recovered
        .jobs
        .iter()
        .find(|stored| stored.id == job.id)
        .unwrap();
    assert_eq!(stored.status, STATUS_READY);
    assert_eq!(stored.current_stage_id, "extract");
    let _ = fs::remove_dir_all(root);
}

#[test]
fn convert_then_translate_records_conversion_and_handoff_artifacts() {
    let root = temp_root("convert-then-translate-success");
    let store = BookPipelineStore::for_test(&root);
    let repo_root = root.join("repo");
    let job = queue_job(
        &store,
        fake_source(None),
        MODE_CONVERT_THEN_TRANSLATE.into(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();
    assert!(job
        .route
        .iter()
        .any(|item| item.route_kind == "translation_handoff" && item.can_run));

    let completed = run_job_with_handoff(
        &store,
        &ArtifactFixtureRunner,
        &FakeTranslationHandoffRunner,
        &job.id,
        Some(&repo_root),
    )
    .unwrap();

    assert_eq!(completed.status, STATUS_READY);
    assert_eq!(completed.current_stage_id, "split");
    assert_eq!(completed.current_step, "Translation handoff ready");
    assert!(completed.last_error.is_none());
    for kind in ["markdown", "html", "epub", "translation_source"] {
        assert!(completed
            .artifacts
            .iter()
            .any(|artifact| artifact.kind == kind));
    }
    let log = completed.log_summary.join("\n");
    assert!(log.contains("fixture runner completed"));
    assert!(log.contains("Conversion completed; translation handoff started"));
    assert!(log.contains("Fake translation handoff ready"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn convert_then_translate_conversion_failure_does_not_run_handoff() {
    let root = temp_root("convert-then-translate-conversion-failure");
    let store = BookPipelineStore::for_test(&root);
    let repo_root = root.join("repo");
    let job = queue_job(
        &store,
        fake_source(None),
        MODE_CONVERT_THEN_TRANSLATE.into(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();

    let failed = run_job_with_handoff(
        &store,
        &ConversionFailingRunner,
        &FakeTranslationHandoffRunner,
        &job.id,
        Some(&repo_root),
    )
    .unwrap();

    assert_eq!(failed.status, STATUS_FAILED);
    assert_eq!(failed.current_step, "Conversion failed");
    assert_eq!(
        failed.last_error.as_deref(),
        Some("Fake conversion backend failed")
    );
    assert!(failed.artifacts.is_empty());
    assert!(!failed
        .log_summary
        .iter()
        .any(|line| line.contains("translation handoff")));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn convert_then_translate_handoff_failure_preserves_conversion_artifacts() {
    let root = temp_root("convert-then-translate-handoff-failure");
    let store = BookPipelineStore::for_test(&root);
    let repo_root = root.join("repo");
    let job = queue_job(
        &store,
        fake_source(None),
        MODE_CONVERT_THEN_TRANSLATE.into(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();

    let failed = run_job_with_handoff(
        &store,
        &ArtifactFixtureRunner,
        &FailingTranslationHandoffRunner,
        &job.id,
        Some(&repo_root),
    )
    .unwrap();

    assert_eq!(failed.status, STATUS_FAILED);
    assert_eq!(failed.current_step, "Translation handoff failed");
    assert_eq!(
        failed.last_error.as_deref(),
        Some("Fake translation handoff failed")
    );
    for kind in ["markdown", "html", "epub"] {
        assert!(failed
            .artifacts
            .iter()
            .any(|artifact| artifact.kind == kind));
    }
    assert!(!failed
        .artifacts
        .iter()
        .any(|artifact| artifact.kind == "translation_project"));
    let log = failed.log_summary.join("\n");
    assert!(log.contains("fixture runner completed"));
    assert!(log.contains("Translation handoff failed: Fake translation handoff failed"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn cleanup_preview_blocks_unsafe_candidate_missing_zotero_child() {
    let root = temp_root("cleanup-unsafe");
    let store = BookPipelineStore::for_test(&root);
    let job = cleanup_fixture_job(&root, &store, None);

    let preview = preview_cleanup_candidates(&store).unwrap();

    assert_eq!(preview.candidates.len(), 1);
    let candidate = &preview.candidates[0];
    assert_eq!(candidate.job_id, job.id);
    assert!(!candidate.can_approve);
    assert!(candidate.checks.iter().any(|check| {
        check.kind == "zotero_child_attachment"
            && !check.ok
            && check.detail.contains("Missing Zotero")
    }));
    let error = approve_cleanup_candidate(&store, &candidate.id, true).unwrap_err();
    assert!(error.contains("zotero_child_attachment"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn cleanup_approval_requires_explicit_safe_candidate() {
    let root = temp_root("cleanup-safe");
    let store = BookPipelineStore::for_test(&root);
    let job = cleanup_fixture_job(&root, &store, Some("MDKEY123"));
    let source_pdf = job.source.path.as_deref().map(PathBuf::from).unwrap();
    let preview = preview_cleanup_candidates(&store).unwrap();
    let candidate = preview.candidates[0].clone();

    // This fixture never built or validated a reading output, so it is not a
    // candidate at all: the evidence used to ignore every stage status, and
    // a book whose validation never ran offered up its source PDF anyway.
    assert!(!candidate.can_approve);
    assert!(candidate
        .checks
        .iter()
        .any(|check| check.kind == "validated_reading" && !check.ok));
    let blocked = approve_cleanup_candidate(&store, &candidate.id, true).unwrap_err();
    assert!(blocked.contains("validated_reading"), "got: {blocked}");
    assert!(approve_cleanup_candidate(&store, &candidate.id, false)
        .unwrap_err()
        .contains("Explicit cleanup approval"));
    assert!(source_pdf.is_file(), "nothing may be deleted either way");
    let _ = fs::remove_dir_all(root);
}

/// A book that really did build and validate a reading output. Queued
/// normally, then decorated, so the evidence logic is exercised against a
/// job shaped the way the pipeline shapes them.
fn cleanup_ready_job(root: &Path, store: &MemoryStateStore) -> BookPipelineJob {
    let output_dir = root.join("reading-output");
    fs::create_dir_all(&output_dir).unwrap();
    let markdown = output_dir.join("book.md");
    fs::write(&markdown, "# Clean Markdown\n").unwrap();
    let epub = output_dir.join("book.epub");
    fs::write(&epub, "epub bytes").unwrap();
    let source_pdf = root.join("source.pdf");
    fs::write(&source_pdf, "%PDF fixture").unwrap();
    let mut source = fake_direct_zotero_source();
    source.path = Some(display_path(&source_pdf));

    let job = queue_job(
        store,
        source,
        "conversion_only".into(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();
    let mut state = store.load().unwrap();
    let stored = state
        .jobs
        .iter_mut()
        .find(|stored| stored.id == job.id)
        .unwrap();
    stored.status = STATUS_COMPLETED.into();
    stored.output_dir = Some(display_path(&output_dir));
    stored.artifacts = vec![BookPipelineArtifact {
        kind: "markdown".into(),
        path: display_path(&markdown),
        sha256: Some(sha256_file(&markdown).unwrap()),
        zotero_key: Some("MDKEY123".into()),
        ..BookPipelineArtifact::default()
    }];
    let child = &mut stored.children[0];
    child.stages.push(BookPipelineStage {
        stage_id: "validate_reading".into(),
        status: STATUS_COMPLETED.into(),
        ..BookPipelineStage::default()
    });
    child.artifacts = vec![
        required_stage_artifact("reading_epub", &epub, "build_reading").unwrap(),
        required_stage_artifact("reading_markdown", &markdown, "build_reading").unwrap(),
    ];
    let ready = stored.clone();
    store.save(&state).unwrap();
    ready
}

// The evidence accepted `html`/`epub`/`translation_source` — conversion-stage
// outputs — and never looked at a stage status, so a book that never built a
// reading output counted as safe to delete the source PDF for.
#[test]
fn cleanup_evidence_requires_a_validated_reading_build() {
    let root = temp_root("cleanup-evidence-reading");
    let store = MemoryStateStore::new(&root);
    let ready = cleanup_ready_job(&root, &store);
    let candidate = cleanup_candidate_for_job(&ready).unwrap();
    assert!(
        candidate.can_approve,
        "a validated reading build is the case that should pass: {:?}",
        candidate.checks
    );

    // Same book, validation not completed.
    let mut unvalidated = ready.clone();
    stage_mut(&mut unvalidated.children[0], "validate_reading")
        .unwrap()
        .status = STATUS_FAILED.into();
    let candidate = cleanup_candidate_for_job(&unvalidated).unwrap();
    assert!(!candidate.can_approve);
    assert!(candidate
        .checks
        .iter()
        .any(|check| check.kind == "validated_reading" && !check.ok));

    // Same book, conversion-stage deliverables only.
    let mut conversion_only = ready.clone();
    conversion_only.output_dir = None;
    for artifact in &mut conversion_only.children[0].artifacts {
        artifact.kind = artifact.kind.replace("reading_", "");
    }
    let candidate = cleanup_candidate_for_job(&conversion_only).unwrap();
    assert!(
        !candidate.can_approve,
        "conversion output is not a built book: {:?}",
        candidate.checks
    );

    let _ = fs::remove_dir_all(root);
}

// The approval only ever existed as a log line, and the log is a ring buffer
// that trims. A decision to delete someone's source PDF has to outlive it.
#[test]
fn cleanup_approval_persists_as_a_bound_record() {
    let root = temp_root("cleanup-approval-record");
    let store = MemoryStateStore::new(&root);
    cleanup_ready_job(&root, &store);
    let candidate = preview_cleanup_candidates(&store).unwrap().candidates[0].clone();

    approve_cleanup_candidate(&store, &candidate.id, true).unwrap();

    let stored = store.load().unwrap().jobs[0].clone();
    let approval = stored
        .approval_references
        .iter()
        .find(|approval| approval.gate_id == CLEANUP_GATE_ID)
        .expect("the decision must be a record, not a log line");
    assert_eq!(approval.decision, "approved");
    assert!(!approval.approval_id.is_empty());
    assert!(!approval.decided_at.is_empty());
    assert_eq!(
        approval.bound_artifact_hashes,
        cleanup_bound_artifact_hashes(&stored),
        "the approval binds the reading artifacts it was taken against"
    );
    assert!(cleanup_approval_is_current(&stored));

    // Trimming the log must not take the record with it.
    let mut state = store.load().unwrap();
    state.jobs[0].log_summary.clear();
    store.save(&state).unwrap();
    assert!(cleanup_approval_is_current(&store.load().unwrap().jobs[0]));

    // The out-of-band deletion path asks this question before deleting.
    let selector = stored.source.selector.clone().unwrap();
    let status = cleanup_approval_status(&store.load().unwrap(), &selector);
    assert!(status.approved, "{}", status.reason);
    assert_eq!(status.approval_id, approval.approval_id);

    // Rebuild the book and the approval stops applying.
    let mut state = store.load().unwrap();
    for artifact in &mut state.jobs[0].children[0].artifacts {
        artifact.sha256 = Some("0".repeat(64));
    }
    store.save(&state).unwrap();
    let state = store.load().unwrap();
    assert!(!cleanup_approval_is_current(&state.jobs[0]));
    assert!(!cleanup_approval_status(&state, &selector).approved);
    assert!(cleanup_approval_status(&state, "no-such-source")
        .reason
        .contains("No Book Pipeline job"));

    let _ = fs::remove_dir_all(root);
}

fn queued_collection_with_a_held_book(
    store: &BookPipelineStore,
) -> (BookPipelineJob, String, String) {
    let job = queue_job(
        store,
        fake_collection_source(),
        "conversion_only".into(),
        BookPipelinePreviewConfig {
            has_paddleocr_credentials: true,
            has_mineru_credentials: true,
            route_overrides: BTreeMap::new(),
        },
    )
    .unwrap();
    let child = job
        .children
        .iter()
        .find(|child| {
            child
                .route
                .iter()
                .any(|item| item.route_kind == "blocked_dirty_text_layer")
        })
        .expect("the fixture collection holds one dirty-text-layer book");
    let route_item_id = child
        .route
        .iter()
        .find(|item| item.route_kind == "blocked_dirty_text_layer")
        .unwrap()
        .id
        .clone();
    (job.clone(), child.id.clone(), route_item_id)
}

fn credentialed_config() -> BookPipelinePreviewConfig {
    BookPipelinePreviewConfig {
        has_paddleocr_credentials: true,
        has_mineru_credentials: true,
        route_overrides: BTreeMap::new(),
    }
}

// The Overview tab offered the wizard's three choices with every button
// disabled, so a held book could only be dealt with by deleting it — which
// for a collection took the whole batch.
#[test]
fn a_held_book_can_be_rerouted_in_place() {
    let root = temp_root("route-override-in-place");
    let store = BookPipelineStore::for_test(&root);
    let (job, child_id, route_item_id) = queued_collection_with_a_held_book(&store);

    let rerouted = set_route_override(
        &store,
        &job.id,
        Some(&child_id),
        &route_item_id,
        "paddle",
        &credentialed_config(),
    )
    .unwrap();

    let child = rerouted
        .children
        .iter()
        .find(|child| child.id == child_id)
        .unwrap();
    let item = child
        .route
        .iter()
        .find(|item| item.id == route_item_id)
        .unwrap();
    assert_eq!(item.route_kind, "remote_paddleocr");
    assert!(item.can_run);
    assert_eq!(item.route_override.as_deref(), Some("paddle"));
    assert_eq!(stage_ref(child, "route").unwrap().status, STATUS_READY);
    assert_eq!(stage_ref(child, "extract").unwrap().status, STATUS_PENDING);
    assert!(child.last_error.is_none());

    // The decision has to survive a restart, so it lives on the source the
    // runner re-routes from, not only on the route it just recomputed.
    let reloaded = store
        .load()
        .unwrap()
        .jobs
        .into_iter()
        .find(|stored| stored.id == job.id)
        .unwrap();
    let child = reloaded
        .children
        .iter()
        .find(|child| child.id == child_id)
        .unwrap();
    assert_eq!(
        child.source.route_overrides.get(&route_item_id),
        Some(&"paddle".to_string())
    );
    assert_eq!(stage_ref(child, "route").unwrap().status, STATUS_READY);

    // `auto` drops the override and schedules a fresh automatic routing pass,
    // which is the only thing that can undo a forced route.
    let cleared = set_route_override(
        &store,
        &job.id,
        Some(&child_id),
        &route_item_id,
        "auto",
        &credentialed_config(),
    )
    .unwrap();
    let child = cleared
        .children
        .iter()
        .find(|child| child.id == child_id)
        .unwrap();
    assert!(child.source.route_overrides.is_empty());
    assert_eq!(stage_ref(child, "route").unwrap().status, STATUS_READY);
    assert_eq!(stage_ref(child, "extract").unwrap().status, STATUS_PENDING);

    let _ = fs::remove_dir_all(root);
}

// Forcing a provider whose credentials are missing must leave the book held,
// not ready a stage that would only fail at the next spawn.
#[test]
fn a_reroute_without_credentials_keeps_the_book_held() {
    let root = temp_root("route-override-no-credentials");
    let store = BookPipelineStore::for_test(&root);
    let (job, child_id, route_item_id) = queued_collection_with_a_held_book(&store);

    let held = set_route_override(
        &store,
        &job.id,
        Some(&child_id),
        &route_item_id,
        "paddle",
        &BookPipelinePreviewConfig::default(),
    )
    .unwrap();

    let child = held
        .children
        .iter()
        .find(|child| child.id == child_id)
        .unwrap();
    let item = child
        .route
        .iter()
        .find(|item| item.id == route_item_id)
        .unwrap();
    assert_eq!(item.route_kind, "missing_credentials");
    assert!(!item.can_run);
    assert_eq!(stage_ref(child, "route").unwrap().status, STATUS_BLOCKED);
    assert_eq!(stage_ref(child, "extract").unwrap().status, STATUS_PENDING);
    assert!(child.last_error.is_some());

    for (item_id, token) in [
        (route_item_id.as_str(), "definitely-not-a-token"),
        ("no-such-route-item", "paddle"),
    ] {
        assert!(
            set_route_override(
                &store,
                &job.id,
                Some(&child_id),
                item_id,
                token,
                &credentialed_config(),
            )
            .is_err(),
            "{item_id}/{token} should be rejected"
        );
    }

    let _ = fs::remove_dir_all(root);
}

#[test]
fn zotero_preview_blocks_only_items_that_need_credentials_or_manual_review() {
    let source = fake_collection_source();

    let route = preview_route(
        &source,
        "conversion_only",
        BookPipelinePreviewConfig {
            has_paddleocr_credentials: false,
            has_mineru_credentials: true,
            route_overrides: BTreeMap::new(),
        },
    );

    assert!(route
        .iter()
        .any(|item| item.route_kind == "direct_text" && item.can_run));
    assert!(route
        .iter()
        .any(|item| item.route_kind == "missing_credentials" && !item.can_run));
    assert!(route
        .iter()
        .any(|item| item.route_kind == "mineru" && item.can_run));
    assert!(route.iter().any(|item| {
        item.route_kind == "blocked_dirty_text_layer"
            && !item.can_run
            && item.blocked_reason.is_some()
    }));
    assert!(route
        .iter()
        .any(|item| item.route_kind == "already_converted" && !item.can_run));

    let route_with_remote_ocr = preview_route(
        &source,
        "conversion_only",
        BookPipelinePreviewConfig {
            has_paddleocr_credentials: true,
            has_mineru_credentials: true,
            route_overrides: BTreeMap::new(),
        },
    );
    assert!(route_with_remote_ocr
        .iter()
        .any(|item| item.route_kind == "remote_paddleocr" && item.can_run));
}

// ---- Staged-gates runner: split + prepare slice (issue #38) ----

fn handoff_repo_fixture(root: &Path) -> PathBuf {
    let repo = root.join("repo");
    fs::create_dir_all(repo.join("tools")).unwrap();
    fs::write(repo.join("AGENTS.md"), "# fixture\n").unwrap();
    fs::write(
        repo.join("tools").join("create_local_book_project.py"),
        "# fixture\n",
    )
    .unwrap();
    repo
}

fn handoff_ready_child_job(
    store: &BookPipelineStore,
    repo: &Path,
    source_path: &Path,
    source_text: &str,
) -> String {
    fs::write(source_path, source_text).unwrap();
    let job = queue_job(
        store,
        markdown_source(source_path),
        MODE_TRANSLATE_ONLY.into(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();
    let handed_off = run_job_with_handoff(
        store,
        &SystemPipelineRunner,
        &LocalProjectHandoffRunner,
        &job.id,
        Some(repo),
    )
    .unwrap();
    assert_eq!(handed_off.current_stage_id, "split");
    job.id
}

#[test]
fn mineru_handoff_preserves_assets_and_split_keeps_links_resolvable() {
    let root = temp_root("mineru-handoff-assets");
    let repo = handoff_repo_fixture(&root);
    let source_path = root.join("source.md");
    let source_assets = root.join("source.mineru");
    fs::create_dir_all(source_assets.join("images")).unwrap();
    fs::create_dir_all(source_assets.join("parts/0001")).unwrap();
    fs::write(source_assets.join("images/figure.png"), b"png-fixture").unwrap();
    fs::write(
        source_assets.join("parts/0001/part.md"),
        "# Wrong per-part candidate\n",
    )
    .unwrap();
    let store = BookPipelineStore::for_test(&root);
    let job_id = handoff_ready_child_job(
        &store,
        &repo,
        &source_path,
        "# One\n\n![Figure](source.mineru/images/figure.png)\n",
    );

    let handed_off = store.load().unwrap().jobs[0].clone();
    let project_root = child_project_root(&handed_off);
    let copied_asset = project_root.join("source/source.mineru/images/figure.png");
    assert_eq!(fs::read(&copied_asset).unwrap(), b"png-fixture");
    assert!(fs::read_to_string(project_root.join("source/source.md"))
        .unwrap()
        .starts_with("# One"));
    assert!(handed_off
        .artifacts
        .iter()
        .all(|artifact| { !artifact.path.contains("source.mineru/parts/0001/part.md") }));

    let split = advance_job(&store, &job_id, None, false).unwrap();
    let chapter_path = split.children[0]
        .artifacts
        .iter()
        .find(|artifact| artifact.kind == "chapter_source")
        .map(|artifact| PathBuf::from(&artifact.path))
        .unwrap();
    let chapter = fs::read_to_string(&chapter_path).unwrap();
    let relative = "../../source/source.mineru/images/figure.png";
    assert!(chapter.contains(relative), "{chapter}");
    assert_eq!(
        chapter_path
            .parent()
            .unwrap()
            .join(relative)
            .canonicalize()
            .unwrap(),
        copied_asset.canonicalize().unwrap()
    );
    let _ = fs::remove_dir_all(root);
}

const PADDLE_ASSET_MARKDOWN: &str =
    "# Sample Book\n\nChapter One\n\n![Figure](Sample_Book_assets/figure.png)\n";

/// Writes the layout `packages/ocr/scripts/pdf_to_html_paddleocr.py` produces:
/// the cleaned Markdown beside its `<stem>_assets` directory, with the image
/// references inside the Markdown pointing at that directory relatively.
struct PaddleAssetsLayoutExecutor;

impl RunnerCommandExecutor for PaddleAssetsLayoutExecutor {
    fn execute(&self, command: &RunnerCommand) -> Result<RunnerCommandResult, String> {
        assert_eq!(command.label, "local PDF conversion wrapper");
        let book_dir = command.output_dir.join("Sample_Book");
        let assets_dir = book_dir.join("Sample_Book_assets");
        fs::create_dir_all(&assets_dir).unwrap();
        fs::write(book_dir.join("Sample_Book.md"), PADDLE_ASSET_MARKDOWN).unwrap();
        fs::write(book_dir.join("Sample_Book.html"), "<h1>Sample Book</h1>\n").unwrap();
        fs::write(assets_dir.join("figure.png"), b"png-fixture").unwrap();
        Ok(RunnerCommandResult {
            stdout: String::new(),
            stderr: String::new(),
            log_summary: vec!["Paddle wrapper completed".into()],
        })
    }
}

#[test]
fn paddle_handoff_copies_the_assets_directory_and_keeps_links_resolvable() {
    let root = temp_root("paddle-handoff-assets");
    let input = root.join("input");
    fs::create_dir_all(&input).unwrap();
    fs::write(input.join("Sample Book.pdf"), "%PDF fixture").unwrap();
    let repo = handoff_repo_fixture(&root);
    let wrapper_root = fake_wrapper_root(&root);
    let store = BookPipelineStore::for_test(&root);
    let job = queue_job(
        &store,
        local_pdf_source(&input),
        MODE_CONVERT_THEN_TRANSLATE.into(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();

    let handed_off = run_job_with_handoff(
        &store,
        &CommandPipelineRunner::with_book_ocr_conversion_root(
            PaddleAssetsLayoutExecutor,
            wrapper_root,
        ),
        &LocalProjectHandoffRunner,
        &job.id,
        Some(&repo),
    )
    .unwrap();

    assert_eq!(handed_off.status, STATUS_READY);
    let project_root = child_project_root(&handed_off);

    // The directory keeps its name, because the Markdown references it by that
    // exact relative path.
    let copied_asset = project_root.join("source/Sample_Book_assets/figure.png");
    assert_eq!(
        fs::read(&copied_asset).unwrap(),
        b"png-fixture",
        "the assets directory must travel with the Markdown"
    );

    // The decisive check: the link inside the handed-off source resolves.
    let source_md = project_root.join("source/source.md");
    let referenced = source_md
        .parent()
        .unwrap()
        .join("Sample_Book_assets/figure.png");
    assert_eq!(
        referenced.canonicalize().unwrap(),
        copied_asset.canonicalize().unwrap()
    );

    let manifest =
        fs::read_to_string(project_root.join("metadata").join("source_manifest.json")).unwrap();
    assert!(
        manifest.contains("source/Sample_Book_assets"),
        "the manifest must record where the resources landed: {manifest}"
    );
    let _ = fs::remove_dir_all(root);
}

fn fake_handoff_ready_job(store: &BookPipelineStore, repo: &Path) -> String {
    fake_handoff_ready_job_with_options(store, repo, false, false)
}

fn fake_handoff_ready_job_with_second_pass(
    store: &BookPipelineStore,
    repo: &Path,
    second_pass_enabled: bool,
) -> String {
    fake_handoff_ready_job_with_options(store, repo, second_pass_enabled, false)
}

fn fake_handoff_ready_job_with_text_cleanup(store: &BookPipelineStore, repo: &Path) -> String {
    let translation_intent = serde_json::from_value(serde_json::json!({
        "translationMode": TRANSLATION_MODE_FAST,
        "profileId": "fake-provider-profile",
        "configId": "fake-provider-config",
        "skillIds": [],
        "secondPassEnabled": false,
        "textCleanup": true,
        "digestMode": false,
        "outputFormats": default_output_formats(),
    }))
    .unwrap();
    let job = queue_job_with_translation_intent(
        store,
        fake_source(None),
        MODE_CONVERT_THEN_TRANSLATE.into(),
        translation_intent,
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();
    let handed_off = run_job_with_handoff(
        store,
        &SystemPipelineRunner,
        &LocalProjectHandoffRunner,
        &job.id,
        Some(repo),
    )
    .unwrap();
    assert_eq!(handed_off.current_stage_id, "split");
    job.id
}

fn fake_handoff_ready_job_with_digest(store: &BookPipelineStore, repo: &Path) -> String {
    fake_handoff_ready_job_with_options(store, repo, false, true)
}

fn fake_handoff_ready_job_with_options(
    store: &BookPipelineStore,
    repo: &Path,
    second_pass_enabled: bool,
    digest_mode: bool,
) -> String {
    fake_handoff_ready_job_with_output_formats(
        store,
        repo,
        second_pass_enabled,
        digest_mode,
        default_output_formats(),
    )
}

fn fake_handoff_ready_job_with_output_formats(
    store: &BookPipelineStore,
    repo: &Path,
    second_pass_enabled: bool,
    digest_mode: bool,
    output_formats: Vec<String>,
) -> String {
    let job = queue_job_with_translation_intent(
        store,
        fake_source(None),
        MODE_CONVERT_THEN_TRANSLATE.into(),
        BookPipelineTranslationIntent {
            translation_mode: TRANSLATION_MODE_FAST.into(),
            profile_id: "fake-provider-profile".into(),
            config_id: "fake-provider-config".into(),
            skill_ids: Vec::new(),
            second_pass_enabled,
            text_cleanup: false,
            digest_mode,
            output_formats,
        },
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();
    let handed_off = run_job_with_handoff(
        store,
        &SystemPipelineRunner,
        &LocalProjectHandoffRunner,
        &job.id,
        Some(repo),
    )
    .unwrap();
    assert_eq!(handed_off.current_stage_id, "split");
    job.id
}

fn child_stage_status(job: &BookPipelineJob, stage_id: &str) -> String {
    job.children[0]
        .stages
        .iter()
        .find(|stage| stage.stage_id == stage_id)
        .unwrap_or_else(|| panic!("stage {stage_id} is missing"))
        .status
        .clone()
}

fn child_project_root(job: &BookPipelineJob) -> PathBuf {
    PathBuf::from(
        job.children[0]
            .local_project_root
            .as_deref()
            .expect("registered local project root"),
    )
}

fn fixture_translation(source: &str, unit_id: &str) -> String {
    let mut translated = source
        .lines()
        .map(|line| {
            if let Some(level) = atx_heading_level(line) {
                format!("{} Translated {unit_id}", "#".repeat(level))
            } else if line.trim().is_empty() {
                String::new()
            } else {
                format!("Translated: {line}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    translated.push('\n');
    translated
}

fn configure_expert_job(store: &BookPipelineStore, job_id: &str) {
    let mut state = store.load().unwrap();
    let job = state.jobs.iter_mut().find(|job| job.id == job_id).unwrap();
    job.translation_mode = TRANSLATION_MODE_EXPERT.into();
    job.translation_profile_id = "fake-agent-profile".into();
    job.translation_config_id = "fake-agent-config".into();
    job.translation_skill_ids = vec![EXPERT_QA_SKILL_ID.into()];
    job.updated_at = now_label();
    derive_job(job);
    store.save(&state).unwrap();
}

fn approve_ready_translation_for_test(store: &BookPipelineStore, job_id: &str) {
    let mut state = store.load().unwrap();
    let job_index = state.jobs.iter().position(|job| job.id == job_id).unwrap();
    assert!(approve_translation_gate(&mut state.jobs[job_index], 0));
    state.jobs[job_index].updated_at = now_label();
    derive_job(&mut state.jobs[job_index]);
    store.save(&state).unwrap();
}

fn approve_ready_promotion_for_test(store: &BookPipelineStore, job_id: &str) -> String {
    let mut state = store.load().unwrap();
    let job_index = state.jobs.iter().position(|job| job.id == job_id).unwrap();
    assert!(approve_promotion_gate(&mut state.jobs[job_index], 0));
    let approval_id = state.jobs[job_index].children[0]
        .stages
        .iter()
        .find(|stage| stage.stage_id == "approve_promotion")
        .and_then(|stage| stage.approval_id.clone())
        .unwrap();
    state.jobs[job_index].updated_at = now_label();
    derive_job(&mut state.jobs[job_index]);
    store.save(&state).unwrap();
    approval_id
}

fn satisfy_translation_handoff(job: &BookPipelineJob) {
    let project_root = child_project_root(job);
    let handoff: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(
            project_root
                .join("qa")
                .join("handoffs")
                .join("translate.json"),
        )
        .unwrap(),
    )
    .unwrap();
    for unit in handoff["units"].as_array().unwrap() {
        let unit_id = unit["unitId"].as_str().unwrap();
        let source =
            fs::read_to_string(project_root.join(unit["sourceChapterPath"].as_str().unwrap()))
                .unwrap();
        let output_path = project_root.join(unit["outputPath"].as_str().unwrap());
        fs::create_dir_all(output_path.parent().unwrap()).unwrap();
        fs::write(output_path, fixture_translation(&source, unit_id)).unwrap();
    }
}

fn qa_handoff(job: &BookPipelineJob) -> ExpertQaHandoff {
    let project_root = child_project_root(job);
    serde_json::from_str(
        &fs::read_to_string(
            project_root
                .join("qa")
                .join("handoffs")
                .join("expert_qa.json"),
        )
        .unwrap(),
    )
    .unwrap()
}

fn set_expert_review(job: &BookPipelineJob, unit_id: &str, status: &str, unresolved: u64) {
    let project_root = child_project_root(job);
    let handoff = qa_handoff(job);
    let control_path = project_root
        .join("qa")
        .join("chapter_controls")
        .join(format!("{unit_id}.json"));
    let mut control: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&control_path).unwrap()).unwrap();
    let translation_sha256 = handoff.translation_hashes[unit_id].clone();
    let skill_ids = handoff.skill_ids.clone();
    control["unresolvedPolysemy"] = serde_json::json!(unresolved);
    control["expertReview"] = serde_json::json!({
        "required": true,
        "status": status,
        "translationSha256": translation_sha256,
        "skillIds": skill_ids,
        "unresolved": {
            "fidelity": unresolved,
            "terminology": 0,
            "note": 0,
            "traceability": 0,
            "polysemy": unresolved,
        },
    });
    fs::write(
        control_path,
        serde_json::to_string_pretty(&control).unwrap() + "\n",
    )
    .unwrap();
}

fn satisfy_qa_handoff(job: &BookPipelineJob) {
    let handoff = qa_handoff(job);
    for unit_id in handoff_sample_ids(&handoff) {
        set_expert_review(job, &unit_id, "pass", 0);
    }
}

fn fake_job_waiting_for_expert_qa(
    store: &BookPipelineStore,
    repo: &Path,
    executor: &dyn RunnerCommandExecutor,
) -> (String, BookPipelineJob) {
    fake_job_waiting_for_expert_qa_with_digest(store, repo, executor, false)
}

fn fake_job_waiting_for_expert_qa_with_digest(
    store: &BookPipelineStore,
    repo: &Path,
    executor: &dyn RunnerCommandExecutor,
    digest_mode: bool,
) -> (String, BookPipelineJob) {
    let job_id = if digest_mode {
        fake_handoff_ready_job_with_digest(store, repo)
    } else {
        fake_handoff_ready_job(store, repo)
    };
    advance_job_with_executor(store, &job_id, None, false, executor).unwrap();
    advance_job_with_executor(store, &job_id, None, false, executor).unwrap();
    let waiting = advance_job_with_executor(store, &job_id, None, false, executor).unwrap();
    assert_eq!(child_stage_status(&waiting, "expert_qa"), STATUS_BLOCKED);
    (job_id, waiting)
}

fn chapter_control(job: &BookPipelineJob, unit_id: &str) -> serde_json::Value {
    let project_root = child_project_root(job);
    serde_json::from_str(
        &fs::read_to_string(
            project_root
                .join("qa")
                .join("chapter_controls")
                .join(format!("{unit_id}.json")),
        )
        .unwrap(),
    )
    .unwrap()
}

#[test]
fn automated_qa_covers_placeholders_structure_glossary_and_completeness() {
    let unit = |translation_text: &str| ExpertQaUnit {
        unit_id: "chapter_001".into(),
        source_text: "# Heading\n\nUse {name} Foo.\n".into(),
        translation_text: translation_text.into(),
        translation_path: PathBuf::new(),
        translation_sha256: sha256_str(translation_text),
        control_path: PathBuf::new(),
    };
    let terms = vec![("Foo".to_string(), "术语".to_string())];

    assert!(automated_qa_checks(&unit("# 标题\n\n使用 {name} 术语。\n"), &terms).passed());
    assert!(
        automated_qa_checks(&unit("# 标题\n\n使用 {name} 术语。\\(^{12}\\)\n"), &terms)
            .placeholder_integrity
    );
    assert!(
        !automated_qa_checks(&unit("# 标题\n\n使用 {name} 术语。^{missing}\n"), &terms)
            .placeholder_integrity
    );
    assert_eq!(
        placeholder_tokens(r"Use ^{name}"),
        BTreeMap::from([(String::from("{name}"), 1)])
    );
    assert!(placeholder_tokens(r"Use \(^{12}\)").is_empty());
    assert!(!automated_qa_checks(&unit("# 标题\n\n使用术语。\n"), &terms).placeholder_integrity);
    assert!(!automated_qa_checks(&unit("## 标题\n\n使用 {name} 术语。\n"), &terms).structure);
    assert!(
        !automated_qa_checks(&unit("# 标题\n\n使用 {name}。\n"), &terms).terminology_consistency
    );
    assert!(!automated_qa_checks(&unit("# 标题\n"), &terms).completeness);
}

#[test]
fn advance_runs_split_and_keeps_prepare_runnable() {
    let root = temp_root("advance-split");
    let repo = handoff_repo_fixture(&root);
    let source_path = root.join("source.md");
    let store = BookPipelineStore::for_test(&root);
    let job_id = handoff_ready_child_job(
        &store,
        &repo,
        &source_path,
        "# Chapter One\n\nAlpha paragraph.\n\nBeta paragraph.\n\n# Chapter Two\n\nGamma paragraph.\n",
    );

    let advanced = advance_job(&store, &job_id, None, false).unwrap();

    assert_eq!(child_stage_status(&advanced, "split"), STATUS_COMPLETED);
    // Prepare is left PENDING but runnable; the gate slice owns readying it.
    assert_eq!(child_stage_status(&advanced, "prepare"), STATUS_PENDING);

    let child = &advanced.children[0];
    let source_map = child
        .artifacts
        .iter()
        .find(|artifact| artifact.kind == "source_map")
        .expect("source_map artifact registered");
    assert!(source_map.sha256.is_some());
    assert_eq!(
        child
            .artifacts
            .iter()
            .filter(|artifact| artifact.kind == "chapter_source")
            .count(),
        2
    );

    let split = child
        .stages
        .iter()
        .find(|stage| stage.stage_id == "split")
        .unwrap();
    assert!(split.input_hashes.contains_key("sourceMarkdownSha256"));

    let project_root = child_project_root(&advanced);
    let chapter_one = project_root
        .join("chapters")
        .join("src")
        .join("chapter_001.md");
    assert!(chapter_one.is_file());
    assert!(fs::read_to_string(&chapter_one)
        .unwrap()
        .contains("Chapter One"));
    assert!(project_root
        .join("metadata")
        .join("source_map.json")
        .is_file());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn advance_runs_prepare_and_parks_before_translation_gate() {
    let root = temp_root("advance-prepare");
    let repo = handoff_repo_fixture(&root);
    let source_path = root.join("source.md");
    let store = BookPipelineStore::for_test(&root);
    let job_id = handoff_ready_child_job(
        &store,
        &repo,
        &source_path,
        "# Alpha\n\nFirst body paragraph.\n\n# Beta\n\nSecond body paragraph.\n",
    );

    advance_job(&store, &job_id, None, false).unwrap();
    let advanced = advance_job(&store, &job_id, None, false).unwrap();

    assert_eq!(child_stage_status(&advanced, "prepare"), STATUS_COMPLETED);
    // The review packet is ready, but the runner never crosses the gate.
    assert_eq!(
        child_stage_status(&advanced, "approve_translation"),
        STATUS_READY
    );
    assert_eq!(advanced.children[0].current_stage_id, "approve_translation");

    let child = &advanced.children[0];
    let approval_request = child
        .stages
        .iter()
        .find(|stage| stage.stage_id == "approve_translation")
        .unwrap()
        .approval_request
        .as_ref()
        .expect("translation approval request");
    assert_eq!(approval_request.gate_id, "translation_disclosure");
    assert_eq!(approval_request.translation_mode, TRANSLATION_MODE_FAST);
    assert!(!approval_request.second_pass_enabled);
    assert!(!approval_request.text_cleanup);
    assert_eq!(
        approval_request.provider_profile_id.as_deref(),
        Some("fake-provider-profile")
    );
    assert_eq!(approval_request.agent_profile_id, None);
    assert_eq!(approval_request.config_id, "fake-provider-config");
    assert!(approval_request.skill_ids.is_empty());
    assert!(approval_request
        .bound_artifact_hashes
        .contains_key("source_markdown"));
    assert!(approval_request
        .bound_artifact_hashes
        .keys()
        .any(|key| key.starts_with("translation_task_manifest:")));
    assert_eq!(
        child
            .artifacts
            .iter()
            .filter(|artifact| artifact.kind == "translation_task_manifest")
            .count(),
        2
    );
    for kind in ["glossary", "style_profile"] {
        assert!(
            child
                .artifacts
                .iter()
                .any(|artifact| artifact.kind == kind && artifact.sha256.is_some()),
            "missing {kind} artifact"
        );
    }
    assert_eq!(
        child
            .artifacts
            .iter()
            .filter(|artifact| artifact.kind == "chapter_control")
            .count(),
        2
    );

    let prepare = child
        .stages
        .iter()
        .find(|stage| stage.stage_id == "prepare")
        .unwrap();
    for key in [
        "sourceMapSha256",
        "glossarySha256",
        "styleProfileSha256",
        "taskPolicyVersion",
    ] {
        assert!(prepare.input_hashes.contains_key(key), "missing key {key}");
    }

    // Task manifests bind hashes and paths but never embed private source text.
    let project_root = child_project_root(&advanced);
    let task_manifest = fs::read_to_string(
        project_root
            .join("qa")
            .join("tasks")
            .join("chapter_001.json"),
    )
    .unwrap();
    assert!(task_manifest.contains("\"chapterId\": \"chapter_001\""));
    assert!(task_manifest.contains("\"sourceChapterSha256\""));
    assert!(!task_manifest.contains("First body paragraph"));
    let persisted = store.load().unwrap();
    let persisted_job = persisted.jobs.iter().find(|job| job.id == job_id).unwrap();
    assert_eq!(
        child_stage_status(persisted_job, "approve_translation"),
        STATUS_READY
    );

    let original_binding = advanced.children[0]
        .stages
        .iter()
        .find(|stage| stage.stage_id == "approve_translation")
        .unwrap()
        .input_hashes["approvalBindingSha256"]
        .clone();
    let mut toggled = advanced.clone();
    toggled.second_pass_enabled = true;
    assert!(ready_translation_approval_gate(&mut toggled, 0));
    let toggled_gate = toggled.children[0]
        .stages
        .iter()
        .find(|stage| stage.stage_id == "approve_translation")
        .unwrap();
    assert!(
        toggled_gate
            .approval_request
            .as_ref()
            .unwrap()
            .second_pass_enabled
    );
    assert_ne!(
        toggled_gate.input_hashes["approvalBindingSha256"],
        original_binding
    );

    let mut cleanup_toggled = advanced.clone();
    cleanup_toggled.text_cleanup = true;
    assert!(ready_translation_approval_gate(&mut cleanup_toggled, 0));
    let cleanup_gate = cleanup_toggled.children[0]
        .stages
        .iter()
        .find(|stage| stage.stage_id == "approve_translation")
        .unwrap();
    assert!(cleanup_gate.approval_request.as_ref().unwrap().text_cleanup);
    assert_ne!(
        cleanup_gate.input_hashes["approvalBindingSha256"],
        original_binding
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn public_gate_approval_requires_explicit_current_binding() {
    let root = temp_root("public-gate-approval");
    let repo = handoff_repo_fixture(&root);
    let source_path = root.join("source.md");
    let store = BookPipelineStore::for_test(&root);
    let job_id = handoff_ready_child_job(
        &store,
        &repo,
        &source_path,
        "# Chapter\n\nBody paragraph.\n",
    );
    advance_job(&store, &job_id, None, false).unwrap();
    let ready = advance_job(&store, &job_id, None, false).unwrap();
    let child_id = ready.children[0].id.clone();

    let rejected = approve_job_gate(
        &store,
        &job_id,
        Some(&child_id),
        "approve_translation",
        false,
    )
    .unwrap_err();
    assert!(rejected.contains("Explicit"));

    let approved = approve_job_gate(
        &store,
        &job_id,
        Some(&child_id),
        "approve_translation",
        true,
    )
    .unwrap();
    assert_eq!(
        child_stage_status(&approved, "approve_translation"),
        STATUS_COMPLETED
    );
    assert_eq!(child_stage_status(&approved, "translate"), STATUS_READY);
    assert!(approved.approval_references.iter().any(|approval| {
        approval.child_job_id == child_id
            && approval.stage_id == "approve_translation"
            && approval.decision == "approved"
    }));

    let repeated = approve_job_gate(
        &store,
        &job_id,
        Some(&child_id),
        "approve_translation",
        true,
    )
    .unwrap_err();
    assert!(repeated.contains("not ready"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn preflight_sample_manifest_carries_the_run_text_cleanup_and_custom_instructions() {
    // The sample exists so a user can approve the full run on the strength of
    // a few translated passages. It only earns that if it is translated under
    // the same instructions -- previously it carried neither field, so anyone
    // who had set one was judging a preview the real run would not reproduce.
    let root = temp_root("translation-sample-prompt-inputs");
    let repo = handoff_repo_fixture(&root);
    let source_path = root.join("source.md");
    let store = BookPipelineStore::for_test(&root);
    let job_id = handoff_ready_child_job(
        &store,
        &repo,
        &source_path,
        "# One\n\nFirst.\n\n# Two\n\nSecond.\n\n# Three\n\nThird.\n\n# Four\n\nFourth.\n\n# Five\n\nFifth.\n",
    );
    advance_job(&store, &job_id, None, false).unwrap();
    let prepared = advance_job(&store, &job_id, None, false).unwrap();
    let child_id = prepared.children[0].id.clone();

    let custom_instructions = BookPipelineCustomInstructions {
        translation: Some("Use restrained literary Chinese.".into()),
        reflection: Some("Critique anachronistic wording.".into()),
    };
    save_book_custom_instructions(
        &store,
        &job_id,
        Some(&child_id),
        custom_instructions.clone(),
    )
    .unwrap();
    let mut state = store.load().unwrap();
    let job = state.jobs.iter_mut().find(|job| job.id == job_id).unwrap();
    job.text_cleanup = true;
    job.updated_at = now_label();
    store.save(&state).unwrap();

    let executor = TranslationSampleFixtureExecutor::default();
    run_translation_sample_with_executor(
        &store,
        &job_id,
        Some(&child_id),
        "fake-provider-profile",
        "sample-config-a",
        false,
        &executor,
    )
    .unwrap();

    let inputs = executor.prompt_inputs();
    assert_eq!(inputs.len(), 1);
    let (text_cleanup, custom) = &inputs[0];
    assert_eq!(text_cleanup, &serde_json::json!(true));
    // The whole object goes over, matching the run manifest. The engine drops
    // the reflection half on the sample path, which runs no reflection pass.
    assert_eq!(
        custom,
        &serde_json::json!({
            "translation": "Use restrained literary Chinese.",
            "reflection": "Critique anachronistic wording.",
        })
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn preflight_sample_rebinds_the_gate_without_adopting_its_provider() {
    let root = temp_root("translation-preflight-sample");
    let repo = handoff_repo_fixture(&root);
    let source_path = root.join("source.md");
    let store = BookPipelineStore::for_test(&root);
    let job_id = handoff_ready_child_job(
        &store,
        &repo,
        &source_path,
        "# One\n\nFirst.\n\n# Two\n\nSecond.\n\n# Three\n\nThird.\n\n# Four\n\nFourth.\n\n# Five\n\nFifth.\n",
    );
    advance_job(&store, &job_id, None, false).unwrap();
    let prepared = advance_job(&store, &job_id, None, false).unwrap();
    let child_id = prepared.children[0].id.clone();
    let original_binding = prepared.children[0]
        .stages
        .iter()
        .find(|stage| stage.stage_id == "approve_translation")
        .unwrap()
        .input_hashes["approvalBindingSha256"]
        .clone();
    let queued_profile = prepared.translation_profile_id.clone();
    let queued_config = prepared.translation_config_id.clone();
    let executor = TranslationSampleFixtureExecutor::default();

    let first = run_translation_sample_with_executor(
        &store,
        &job_id,
        Some(&child_id),
        "fake-provider-profile",
        "sample-config-a",
        false,
        &executor,
    )
    .unwrap();
    let first_gate = first.children[0]
        .stages
        .iter()
        .find(|stage| stage.stage_id == "approve_translation")
        .unwrap();
    let first_binding = first_gate.input_hashes["approvalBindingSha256"].clone();
    let first_evidence = first_gate
        .approval_request
        .as_ref()
        .unwrap()
        .sample_evidence["translation_sample_report"]
        .clone();
    assert_ne!(first_binding, original_binding);
    assert_eq!(first_gate.status, STATUS_READY);
    assert_eq!(child_stage_status(&first, "translate"), STATUS_PENDING);
    // Sampling is "try before you decide": it must not adopt the provider it
    // was run with. It used to, so one sample silently redirected the book.
    assert_eq!(first.translation_profile_id, queued_profile);
    assert_eq!(first.translation_config_id, queued_config);
    let first_artifact = first.children[0]
        .artifacts
        .iter()
        .find(|artifact| artifact.kind == "translation_sample_report")
        .unwrap();
    assert_eq!(
        first_artifact.sha256.as_deref(),
        Some(first_evidence.as_str())
    );
    assert_eq!(first_artifact.privacy, "private_text");
    let first_report = read_translation_sample_report(&first, &child_id).unwrap();
    assert_eq!(first_report.samples.len(), 3);
    assert_eq!(first_report.samples[1].degradation, "aligned");
    let sample_dir = child_project_root(&first).join("qa").join("sample-compare");
    assert!(Path::new(&first_artifact.path).is_file());
    assert!(fs::read_dir(&sample_dir).unwrap().all(|entry| !entry
        .unwrap()
        .file_name()
        .to_string_lossy()
        .starts_with("manifest-")));
    assert_eq!(fs::read_dir(&sample_dir).unwrap().count(), 1);
    assert!(fs::read_dir(
        child_project_root(&first)
            .join("chapters")
            .join("translated")
    )
    .unwrap()
    .next()
    .is_none());

    let second = run_translation_sample_with_executor(
        &store,
        &job_id,
        Some(&child_id),
        "fake-provider-profile",
        "sample-config-b",
        false,
        &executor,
    )
    .unwrap();
    let second_gate = second.children[0]
        .stages
        .iter()
        .find(|stage| stage.stage_id == "approve_translation")
        .unwrap();
    assert_ne!(
        second_gate.input_hashes["approvalBindingSha256"],
        first_binding
    );
    assert_ne!(
        second_gate
            .approval_request
            .as_ref()
            .unwrap()
            .sample_evidence["translation_sample_report"],
        first_evidence
    );
    assert_eq!(
        second.children[0]
            .artifacts
            .iter()
            .filter(|artifact| artifact.kind == "translation_sample_report")
            .count(),
        1
    );
    assert_eq!(fs::read_dir(&sample_dir).unwrap().count(), 1);
    assert_eq!(
        executor.requests(),
        vec![
            ("fake-provider-profile".into(), "sample-config-a".into()),
            ("fake-provider-profile".into(), "sample-config-b".into()),
        ]
    );

    let approved = approve_job_gate(
        &store,
        &job_id,
        Some(&child_id),
        "approve_translation",
        true,
    )
    .unwrap();
    assert_eq!(
        child_stage_status(&approved, "approve_translation"),
        STATUS_COMPLETED
    );
    assert_eq!(child_stage_status(&approved, "translate"), STATUS_READY);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn adopting_a_sampled_provider_is_a_separate_action_that_rebinds_the_gate() {
    // The counterpart to the test above: sampling leaves the job alone, so
    // there has to be an explicit way to say "translate the book with this
    // one", and taking it must drop an approval granted against the old
    // provider -- the provider is inside the binding the user approved.
    let root = temp_root("translation-provider-adopt");
    let repo = handoff_repo_fixture(&root);
    let source_path = root.join("source.md");
    let store = BookPipelineStore::for_test(&root);
    let job_id = handoff_ready_child_job(
        &store,
        &repo,
        &source_path,
        "# One\n\nFirst.\n\n# Two\n\nSecond.\n",
    );
    advance_job(&store, &job_id, None, false).unwrap();
    let prepared = advance_job(&store, &job_id, None, false).unwrap();
    let child_id = prepared.children[0].id.clone();
    let queued_config = prepared.translation_config_id.clone();
    let original_binding = prepared.children[0]
        .stages
        .iter()
        .find(|stage| stage.stage_id == "approve_translation")
        .unwrap()
        .input_hashes["approvalBindingSha256"]
        .clone();

    let adopted = set_translation_provider_in_store(
        &store,
        &job_id,
        Some(&child_id),
        "fake-provider-profile",
        "adopted-config",
    )
    .unwrap();
    assert_ne!(queued_config, "adopted-config");
    assert_eq!(adopted.translation_profile_id, "fake-provider-profile");
    assert_eq!(adopted.translation_config_id, "adopted-config");
    let gate = adopted.children[0]
        .stages
        .iter()
        .find(|stage| stage.stage_id == "approve_translation")
        .unwrap();
    assert_ne!(gate.input_hashes["approvalBindingSha256"], original_binding);
    assert_eq!(gate.status, STATUS_READY);
    assert_eq!(
        gate.approval_request.as_ref().unwrap().config_id,
        "adopted-config"
    );
    assert_eq!(child_stage_status(&adopted, "translate"), STATUS_PENDING);

    // Setting the same slot again is a no-op rather than a spurious rebind.
    let repeated = set_translation_provider_in_store(
        &store,
        &job_id,
        Some(&child_id),
        "fake-provider-profile",
        "adopted-config",
    )
    .unwrap();
    assert_eq!(
        repeated.children[0]
            .stages
            .iter()
            .find(|stage| stage.stage_id == "approve_translation")
            .unwrap()
            .input_hashes["approvalBindingSha256"],
        gate.input_hashes["approvalBindingSha256"]
    );

    // An approval granted against one provider must not carry over to another.
    let approved = approve_job_gate(
        &store,
        &job_id,
        Some(&child_id),
        "approve_translation",
        true,
    )
    .unwrap();
    assert_eq!(
        child_stage_status(&approved, "approve_translation"),
        STATUS_COMPLETED
    );
    let switched = set_translation_provider_in_store(
        &store,
        &job_id,
        Some(&child_id),
        "fake-provider-profile",
        "another-config",
    )
    .unwrap();
    assert_eq!(
        child_stage_status(&switched, "approve_translation"),
        STATUS_READY
    );
    assert_eq!(child_stage_status(&switched, "translate"), STATUS_PENDING);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn digest_mode_change_reopens_completed_translation_approval() {
    let root = temp_root("digest-mode-approval-recheck");
    let repo = handoff_repo_fixture(&root);
    let source_path = root.join("source.md");
    let store = BookPipelineStore::for_test(&root);
    let job_id = handoff_ready_child_job(
        &store,
        &repo,
        &source_path,
        "# Chapter\n\nBody paragraph.\n",
    );
    advance_job(&store, &job_id, None, false).unwrap();
    advance_job(&store, &job_id, None, false).unwrap();
    approve_ready_translation_for_test(&store, &job_id);
    let approved = store
        .load()
        .unwrap()
        .jobs
        .into_iter()
        .find(|job| job.id == job_id)
        .unwrap();
    let original_gate = approved.children[0]
        .stages
        .iter()
        .find(|stage| stage.stage_id == "approve_translation")
        .unwrap();
    assert_eq!(original_gate.status, STATUS_COMPLETED);
    let original_binding = original_gate.input_hashes["approvalBindingSha256"].clone();
    let original_approval_id = original_gate.approval_id.clone().unwrap();

    let mut toggled = approved;
    toggled.digest_mode = true;
    assert!(ready_translation_approval_gate(&mut toggled, 0));
    let toggled_gate = toggled.children[0]
        .stages
        .iter()
        .find(|stage| stage.stage_id == "approve_translation")
        .unwrap();
    assert_eq!(toggled_gate.status, STATUS_READY);
    assert!(toggled_gate.approval_id.is_none());
    assert_eq!(child_stage_status(&toggled, "build_digest"), STATUS_PENDING);
    assert_ne!(
        toggled_gate.input_hashes["approvalBindingSha256"],
        original_binding
    );
    assert_eq!(
        serde_json::to_value(toggled_gate.approval_request.as_ref().unwrap()).unwrap()
            ["digestMode"],
        true
    );
    assert!(!toggled
        .approval_references
        .iter()
        .any(|approval| approval.approval_id == original_approval_id));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn output_formats_change_reopens_completed_translation_approval() {
    let root = temp_root("output-formats-approval-recheck");
    let repo = handoff_repo_fixture(&root);
    let source_path = root.join("source.md");
    let store = BookPipelineStore::for_test(&root);
    let job_id = handoff_ready_child_job(
        &store,
        &repo,
        &source_path,
        "# Chapter\n\nBody paragraph.\n",
    );
    advance_job(&store, &job_id, None, false).unwrap();
    advance_job(&store, &job_id, None, false).unwrap();
    approve_ready_translation_for_test(&store, &job_id);
    let approved = store
        .load()
        .unwrap()
        .jobs
        .into_iter()
        .find(|job| job.id == job_id)
        .unwrap();
    let original_gate = approved.children[0]
        .stages
        .iter()
        .find(|stage| stage.stage_id == "approve_translation")
        .unwrap();
    assert_eq!(original_gate.status, STATUS_COMPLETED);
    let original_binding = original_gate.input_hashes["approvalBindingSha256"].clone();
    let original_approval_id = original_gate.approval_id.clone().unwrap();

    let mut toggled = approved;
    toggled.output_formats.push(OUTPUT_FORMAT_BILINGUAL.into());
    assert!(ready_translation_approval_gate(&mut toggled, 0));
    let toggled_gate = toggled.children[0]
        .stages
        .iter()
        .find(|stage| stage.stage_id == "approve_translation")
        .unwrap();
    assert_eq!(toggled_gate.status, STATUS_READY);
    assert!(toggled_gate.approval_id.is_none());
    assert_ne!(
        toggled_gate.input_hashes["approvalBindingSha256"],
        original_binding
    );
    assert_eq!(
        toggled_gate
            .approval_request
            .as_ref()
            .unwrap()
            .output_formats,
        vec!["md", "html", "epub", "bilingual"]
    );
    assert!(!toggled
        .approval_references
        .iter()
        .any(|approval| approval.approval_id == original_approval_id));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn custom_instructions_change_reopens_completed_translation_approval() {
    let root = temp_root("custom-instructions-approval-recheck");
    let repo = handoff_repo_fixture(&root);
    let source_path = root.join("source.md");
    let store = BookPipelineStore::for_test(&root);
    let job_id = handoff_ready_child_job(
        &store,
        &repo,
        &source_path,
        "# Chapter\n\nBody paragraph.\n",
    );
    advance_job(&store, &job_id, None, false).unwrap();
    advance_job(&store, &job_id, None, false).unwrap();
    approve_ready_translation_for_test(&store, &job_id);
    let approved = store
        .load()
        .unwrap()
        .jobs
        .into_iter()
        .find(|job| job.id == job_id)
        .unwrap();
    let original_gate = approved.children[0]
        .stages
        .iter()
        .find(|stage| stage.stage_id == "approve_translation")
        .unwrap();
    let original_binding = original_gate.input_hashes["approvalBindingSha256"].clone();
    let original_approval_id = original_gate.approval_id.clone().unwrap();

    let updated = save_book_custom_instructions(
        &store,
        &job_id,
        None,
        BookPipelineCustomInstructions {
            translation: Some("Use restrained literary Chinese.".into()),
            reflection: None,
        },
    )
    .unwrap();
    let updated_gate = updated.children[0]
        .stages
        .iter()
        .find(|stage| stage.stage_id == "approve_translation")
        .unwrap();
    assert_eq!(updated_gate.status, STATUS_READY);
    assert!(updated_gate.approval_id.is_none());
    assert!(updated_gate
        .input_hashes
        .contains_key("customInstructionsSha256"));
    assert_ne!(
        updated_gate.input_hashes["approvalBindingSha256"],
        original_binding
    );
    assert!(!updated
        .approval_references
        .iter()
        .any(|approval| approval.approval_id == original_approval_id));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn fast_book_passes_enabled_second_pass_to_translation_manifest() {
    let root = temp_root("fake-translate-second-pass");
    let repo = handoff_repo_fixture(&root);
    let store = BookPipelineStore::for_test(&root);
    let job_id = fake_handoff_ready_job_with_second_pass(&store, &repo, true);
    let executor = TranslationEngineFixtureExecutor::with_second_pass_enabled();

    advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    let advanced = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();

    assert!(advanced.second_pass_enabled);
    assert_eq!(child_stage_status(&advanced, "translate"), STATUS_COMPLETED);
    assert_eq!(
        executor.requested_units(),
        vec![vec!["chapter_001".to_string()]]
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn fast_book_passes_enabled_text_cleanup_to_translation_manifest() {
    let root = temp_root("fake-translate-text-cleanup");
    let repo = handoff_repo_fixture(&root);
    let store = BookPipelineStore::for_test(&root);
    let job_id = fake_handoff_ready_job_with_text_cleanup(&store, &repo);
    let executor = TranslationEngineFixtureExecutor::with_text_cleanup();

    advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    let advanced = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();

    assert!(advanced.text_cleanup);
    assert_eq!(child_stage_status(&advanced, "translate"), STATUS_COMPLETED);
    assert_eq!(
        executor.requested_units(),
        vec![vec!["chapter_001".to_string()]]
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn book_custom_instructions_persist_bind_approval_and_flow_to_run_manifest() {
    let root = temp_root("book-custom-instructions");
    let repo = handoff_repo_fixture(&root);
    let store = BookPipelineStore::for_test(&root);
    let job_id = fake_handoff_ready_job_with_second_pass(&store, &repo, true);
    let custom_instructions = BookPipelineCustomInstructions {
        translation: Some("Use restrained literary Chinese.".into()),
        reflection: Some("Critique anachronistic wording.".into()),
    };

    let saved =
        save_book_custom_instructions(&store, &job_id, None, custom_instructions.clone()).unwrap();

    assert_eq!(
        saved.children[0].custom_instructions.as_ref(),
        Some(&custom_instructions)
    );
    let persisted: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&store.state_path).unwrap()).unwrap();
    assert_eq!(
        persisted["jobs"][0]["children"][0]["customInstructions"],
        serde_json::json!({
            "translation": "Use restrained literary Chinese.",
            "reflection": "Critique anachronistic wording.",
        })
    );

    let executor =
        TranslationEngineFixtureExecutor::with_custom_instructions(custom_instructions.clone());
    advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    let advanced = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    let gate = advanced.children[0]
        .stages
        .iter()
        .find(|stage| stage.stage_id == "approve_translation")
        .unwrap();
    assert!(gate.input_hashes.contains_key("customInstructionsSha256"));
    assert_eq!(child_stage_status(&advanced, "translate"), STATUS_COMPLETED);
    let manifest: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(child_project_root(&advanced).join("qa/tasks/run.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        manifest["customInstructions"],
        serde_json::to_value(custom_instructions).unwrap()
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn custom_instructions_are_per_book_and_reject_overlong_text() {
    let root = temp_root("per-book-custom-instructions");
    let store = BookPipelineStore::for_test(&root);
    let job = queue_job_with_translation_intent(
        &store,
        fake_collection_source(),
        MODE_CONVERT_THEN_TRANSLATE.into(),
        fast_translation_intent(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();
    let selected_child_id = job.children[0].id.clone();
    let custom_instructions = BookPipelineCustomInstructions {
        translation: Some("Keep this book's dry humor.".into()),
        reflection: None,
    };

    let saved = save_book_custom_instructions(
        &store,
        &job.id,
        Some(&selected_child_id),
        custom_instructions.clone(),
    )
    .unwrap();

    assert_eq!(
        saved.children[0].custom_instructions.as_ref(),
        Some(&custom_instructions)
    );
    assert!(saved.children[1..]
        .iter()
        .all(|child| child.custom_instructions.is_none()));

    let error = save_book_custom_instructions(
        &store,
        &job.id,
        Some(&selected_child_id),
        BookPipelineCustomInstructions {
            translation: Some("x".repeat(2001)),
            reflection: None,
        },
    )
    .unwrap_err();
    assert!(error.contains("custom_instructions_too_long"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn glossary_violations_reach_the_log_without_failing_the_translation() {
    // The engine report is parsed from stdout and never written anywhere the
    // reader can open, so a metric the runner does not surface is a metric
    // nobody will ever see. This pins the surfacing, and pins that it stays
    // a warning: the chapters are complete and the stage still passes.
    let root = temp_root("glossary-violation-log");
    let repo = handoff_repo_fixture(&root);
    let store = BookPipelineStore::for_test(&root);
    let job_id = fake_handoff_ready_job(&store, &repo);
    let executor = TranslationEngineFixtureExecutor::reporting_glossary_violations(&[
        ("Fan", "风扇"),
        ("Secret", "秘密"),
    ]);

    advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    let advanced = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();

    assert_eq!(child_stage_status(&advanced, "translate"), STATUS_COMPLETED);
    let translate_stage = advanced.children[0]
        .stages
        .iter()
        .find(|stage| stage.stage_id == "translate")
        .unwrap();
    assert!(translate_stage.error.is_none());

    let log = advanced.log_summary.join("\n");
    assert!(log.contains("Glossary check: 2 required term(s) not found"));
    assert!(log.contains("Fan -> 风扇"));
    assert!(log.contains("Secret -> 秘密"));
    assert!(log.contains("complete and unmodified"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_clean_glossary_adds_no_warning_line() {
    let root = temp_root("glossary-violation-absent");
    let repo = handoff_repo_fixture(&root);
    let store = BookPipelineStore::for_test(&root);
    let job_id = fake_handoff_ready_job(&store, &repo);
    let executor = TranslationEngineFixtureExecutor::succeeding();

    advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    let advanced = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();

    assert_eq!(child_stage_status(&advanced, "translate"), STATUS_COMPLETED);
    assert!(!advanced.log_summary.join("\n").contains("Glossary check"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn fake_book_auto_approves_translates_and_stops_at_expert_qa() {
    let root = temp_root("fake-translate-success");
    let repo = handoff_repo_fixture(&root);
    let store = BookPipelineStore::for_test(&root);
    let job_id = fake_handoff_ready_job(&store, &repo);
    let executor = TranslationEngineFixtureExecutor::succeeding();

    advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    let advanced = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();

    assert_eq!(
        child_stage_status(&advanced, "approve_translation"),
        STATUS_COMPLETED
    );
    assert_eq!(child_stage_status(&advanced, "translate"), STATUS_COMPLETED);
    assert_eq!(child_stage_status(&advanced, "expert_qa"), STATUS_PENDING);
    assert_eq!(advanced.current_stage_id, "expert_qa");
    assert_eq!(
        executor.requested_units(),
        vec![vec!["chapter_001".to_string()]]
    );

    let child = &advanced.children[0];
    let gate = child
        .stages
        .iter()
        .find(|stage| stage.stage_id == "approve_translation")
        .unwrap();
    let approval_id = gate.approval_id.as_deref().expect("approval ID");
    let approval = advanced
        .approval_references
        .iter()
        .find(|approval| approval.approval_id == approval_id)
        .expect("approval reference");
    assert_eq!(approval.decision, "approved");
    assert_eq!(
        approval.bound_artifact_hashes,
        gate.approval_request
            .as_ref()
            .unwrap()
            .bound_artifact_hashes
    );

    let translated = child
        .artifacts
        .iter()
        .find(|artifact| artifact.kind == "chapter_translation")
        .expect("translated chapter artifact");
    assert_eq!(translated.producer_stage.as_deref(), Some("translate"));
    let translated_sha256 = sha256_file(Path::new(&translated.path)).unwrap();
    assert_eq!(
        translated.sha256.as_deref(),
        Some(translated_sha256.as_str())
    );
    let translate = child
        .stages
        .iter()
        .find(|stage| stage.stage_id == "translate")
        .unwrap();
    let summary = translate.unit_summary.as_ref().unwrap();
    assert_eq!(
        (summary.total, summary.completed, summary.failed),
        (1, 1, 0)
    );
    assert!(translate.artifact_ids.contains(&translated.artifact_id));
    assert_eq!(translate.attempt, 1);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn fake_fast_book_runs_layered_qa_and_readies_promotion_gate() {
    let root = temp_root("fake-fast-expert-qa");
    let repo = handoff_repo_fixture(&root);
    let store = BookPipelineStore::for_test(&root);
    let job_id = fake_handoff_ready_job(&store, &repo);
    let executor = TranslationEngineFixtureExecutor::succeeding();

    advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    let translated = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    let waiting = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();

    assert_eq!(
        child_stage_status(&translated, "translate"),
        STATUS_COMPLETED
    );
    assert_eq!(child_stage_status(&waiting, "expert_qa"), STATUS_BLOCKED);
    let qa_stage = waiting.children[0]
        .stages
        .iter()
        .find(|stage| stage.stage_id == "expert_qa")
        .unwrap();
    assert!(qa_stage
        .execution_owner
        .as_deref()
        .unwrap()
        .starts_with(AGENT_EXECUTION_OWNER_PREFIX));
    assert!(waiting.children[0]
        .artifacts
        .iter()
        .any(|artifact| artifact.kind == "expert_qa_handoff"));

    satisfy_qa_handoff(&waiting);
    let ready = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();

    assert_eq!(child_stage_status(&ready, "expert_qa"), STATUS_COMPLETED);
    assert_eq!(
        child_stage_status(&ready, "approve_promotion"),
        STATUS_READY
    );
    assert_eq!(ready.current_stage_id, "approve_promotion");
    let request = ready.children[0]
        .stages
        .iter()
        .find(|stage| stage.stage_id == "approve_promotion")
        .unwrap()
        .approval_request
        .as_ref()
        .unwrap();
    assert_eq!(request.gate_id, "promotion");
    assert_eq!(request.qa_policy.as_deref(), Some(TRANSLATION_MODE_FAST));
    assert!(request
        .bound_artifact_hashes
        .keys()
        .any(|key| key.starts_with("chapter_translation:")));
    assert!(request
        .bound_artifact_hashes
        .keys()
        .any(|key| key.starts_with("chapter_control:")));
    assert!(request
        .sample_evidence
        .keys()
        .any(|key| key.starts_with("chapter_control:")));
    let control = chapter_control(&ready, "chapter_001");
    assert_eq!(control["qaPolicy"], TRANSLATION_MODE_FAST);
    assert_eq!(control["checks"]["closure"], "pass");
    assert_eq!(control["unresolvedPolysemy"], 0);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn fake_expert_book_waits_for_both_agent_handoffs_then_readies_promotion_gate() {
    let root = temp_root("fake-expert-handoffs");
    let repo = handoff_repo_fixture(&root);
    let store = BookPipelineStore::for_test(&root);
    let job_id = fake_handoff_ready_job(&store, &repo);
    configure_expert_job(&store, &job_id);
    let executor = TranslationEngineFixtureExecutor::succeeding();

    advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    let prepared = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    assert_eq!(
        child_stage_status(&prepared, "approve_translation"),
        STATUS_READY
    );
    assert_eq!(executor.requested_units(), Vec::<Vec<String>>::new());
    approve_ready_translation_for_test(&store, &job_id);

    let translation_waiting =
        advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    assert_eq!(
        child_stage_status(&translation_waiting, "translate"),
        STATUS_BLOCKED
    );
    assert!(translation_waiting.children[0]
        .artifacts
        .iter()
        .any(|artifact| artifact.kind == "translation_handoff"));
    satisfy_translation_handoff(&translation_waiting);

    let translated = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    assert_eq!(
        child_stage_status(&translated, "translate"),
        STATUS_COMPLETED
    );
    let qa_waiting = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    assert_eq!(child_stage_status(&qa_waiting, "expert_qa"), STATUS_BLOCKED);
    satisfy_qa_handoff(&qa_waiting);

    let ready = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    assert_eq!(child_stage_status(&ready, "expert_qa"), STATUS_COMPLETED);
    assert_eq!(
        child_stage_status(&ready, "approve_promotion"),
        STATUS_READY
    );
    let request = ready.children[0]
        .stages
        .iter()
        .find(|stage| stage.stage_id == "approve_promotion")
        .unwrap()
        .approval_request
        .as_ref()
        .unwrap();
    assert_eq!(request.qa_policy.as_deref(), Some(TRANSLATION_MODE_EXPERT));
    assert_eq!(
        request.agent_profile_id.as_deref(),
        Some("fake-agent-profile")
    );
    assert_eq!(request.skill_ids, vec![EXPERT_QA_SKILL_ID.to_string()]);
    assert_eq!(executor.requested_units(), Vec::<Vec<String>>::new());
    let _ = fs::remove_dir_all(root);
}

// The Stages tab has always labelled a failure "retryable"; until now there
// was no automatic retry behind that word. A stage that keeps failing must
// spend a bounded budget and then say why it stopped.
#[test]
fn a_retryable_stage_failure_is_retried_to_the_budget_then_gives_up() {
    let root = temp_root("stage-retry-budget");
    let repo = handoff_repo_fixture(&root);
    let store = BookPipelineStore::for_test(&root);
    let executor = ReadingPipelineFixtureExecutor::failing_epubcheck();
    let (job_id, waiting) = fake_job_waiting_for_expert_qa(&store, &repo, &executor);
    satisfy_qa_handoff(&waiting);
    advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    approve_ready_promotion_for_test(&store, &job_id);
    advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();

    let failed = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();

    let stage = |job: &BookPipelineJob| {
        job.children[0]
            .stages
            .iter()
            .find(|stage| stage.stage_id == "validate_reading")
            .unwrap()
            .clone()
    };
    let validate = stage(&failed);
    assert_eq!(validate.status, STATUS_FAILED);
    assert_eq!(
        validate.attempt, DEFAULT_STAGE_MAX_ATTEMPTS,
        "the whole budget should be spent before giving up"
    );
    assert_eq!(
        validate.give_up_reason.as_deref(),
        Some(GIVE_UP_RETRY_BUDGET_EXHAUSTED)
    );
    assert!(
        validate.next_retry_at.is_none(),
        "nothing is scheduled once the budget is gone"
    );

    // The same story has to be readable from the progress the UI polls.
    let persisted = store
        .load()
        .unwrap()
        .jobs
        .into_iter()
        .find(|job| job.id == job_id)
        .unwrap();
    assert_eq!(persisted.progress.active_stage_id, "validate_reading");
    assert_eq!(persisted.progress.retry_attempts_remaining, 0);
    assert_eq!(
        persisted.progress.give_up_reason.as_deref(),
        Some(GIVE_UP_RETRY_BUDGET_EXHAUSTED)
    );

    // A give-up written for the automatic loop must not refuse the operator:
    // an advance still runs. It runs *once* — the budget counts the stage's
    // whole life, so clicking Advance cannot spin up a fresh ladder each time.
    let retried = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    assert_eq!(stage(&retried).attempt, DEFAULT_STAGE_MAX_ATTEMPTS + 1);
    assert_eq!(
        stage(&retried).give_up_reason.as_deref(),
        Some(GIVE_UP_RETRY_BUDGET_EXHAUSTED)
    );

    let _ = fs::remove_dir_all(root);
}

// Expert QA blocks on a judgement call, not on a flaky process. Retrying it
// automatically would burn the budget on something no retry can fix.
#[test]
fn a_non_retryable_failure_is_not_retried_automatically() {
    let stage = |code: &str, retryable: bool| BookPipelineStage {
        stage_id: "expert_qa".into(),
        status: STATUS_FAILED.into(),
        attempt: 1,
        safe_error: Some(BookPipelineSafeError {
            code: code.into(),
            summary: "redacted".into(),
            retryable,
            attempt: 1,
            stage_id: "expert_qa".into(),
            ..BookPipelineSafeError::default()
        }),
        ..BookPipelineStage::default()
    };
    let mut child = BookPipelineChildJob {
        stages: vec![stage("qa_blocked", false)],
        ..BookPipelineChildJob::default()
    };

    assert_eq!(schedule_stage_retry(&mut child, "expert_qa"), None);
    let blocked = stage_ref(&child, "expert_qa").unwrap();
    assert_eq!(
        blocked.give_up_reason.as_deref(),
        Some(GIVE_UP_NOT_RETRYABLE)
    );
    assert_eq!(
        blocked.attempt, 1,
        "a non-retryable failure must not spend an attempt"
    );
    assert_eq!(stage_attempts_remaining(blocked), 0);

    // The same stage classified as retryable does schedule one.
    let mut child = BookPipelineChildJob {
        stages: vec![stage("runner_failed", true)],
        ..BookPipelineChildJob::default()
    };
    assert_eq!(
        schedule_stage_retry(&mut child, "expert_qa"),
        Some(DEFAULT_STAGE_RETRY_BACKOFF_SECONDS[0])
    );
    let scheduled = stage_ref(&child, "expert_qa").unwrap();
    assert!(scheduled.give_up_reason.is_none());
    assert!(scheduled.next_retry_at.is_some());
    assert_eq!(
        stage_attempts_remaining(scheduled),
        DEFAULT_STAGE_MAX_ATTEMPTS - 1
    );
}

// A stage that carries its own policy is honoured over the default, which is
// what makes the persisted table worth persisting.
#[test]
fn a_stage_policy_overrides_the_default_budget_and_backoff() {
    let mut child = BookPipelineChildJob {
        stages: vec![BookPipelineStage {
            stage_id: "translate".into(),
            status: STATUS_FAILED.into(),
            attempt: 1,
            max_attempts: 2,
            retry_backoff_seconds: vec![7],
            safe_error: Some(BookPipelineSafeError {
                retryable: true,
                ..BookPipelineSafeError::default()
            }),
            ..BookPipelineStage::default()
        }],
        ..BookPipelineChildJob::default()
    };

    assert_eq!(schedule_stage_retry(&mut child, "translate"), Some(7));

    // A one-entry table repeats its last entry rather than dropping to no wait.
    stage_mut(&mut child, "translate").unwrap().attempt = 2;
    assert_eq!(schedule_stage_retry(&mut child, "translate"), None);
    assert_eq!(
        stage_ref(&child, "translate")
            .unwrap()
            .give_up_reason
            .as_deref(),
        Some(GIVE_UP_RETRY_BUDGET_EXHAUSTED),
        "max_attempts 2 means the second failure is the last"
    );
}

#[test]
fn translate_failure_persists_each_units_safe_reason() {
    let root = temp_root("fake-translate-failure-reason");
    let repo = handoff_repo_fixture(&root);
    let store = BookPipelineStore::for_test(&root);
    let job_id = fake_handoff_ready_job(&store, &repo);
    let handed_off = store
        .load()
        .unwrap()
        .jobs
        .into_iter()
        .find(|job| job.id == job_id)
        .unwrap();
    let project_root = child_project_root(&handed_off);
    fs::write(
        project_root.join("source").join("source.md"),
        "# Alpha\n\nFirst body.\n\n# Beta\n\nSecond body.\n",
    )
    .unwrap();
    let executor =
        TranslationEngineFixtureExecutor::failing_once_with_code("chapter_002", "provider_timeout");
    let mut state = store.load().unwrap();
    let stored_job = state.jobs.iter_mut().find(|job| job.id == job_id).unwrap();
    let translate = stored_job.children[0]
        .stages
        .iter_mut()
        .find(|stage| stage.stage_id == "translate")
        .unwrap();
    translate.max_attempts = 1;
    store.save(&state).unwrap();

    advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    let failed = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();

    let translate = failed.children[0]
        .stages
        .iter()
        .find(|stage| stage.stage_id == "translate")
        .unwrap();
    let summary = translate.unit_summary.as_ref().unwrap();
    assert_eq!(summary.failures.len(), 1);
    assert_eq!(summary.failures[0].unit_id, "chapter_002");
    assert_eq!(summary.failures[0].code, "provider_timeout");
    assert!(summary.failures[0].retryable);
    assert_eq!(
        translate.error.as_deref(),
        Some("Translation failed for 1 unit(s). See failed-unit details.")
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn translate_failure_retries_only_failed_units_and_recovers_stage() {
    let root = temp_root("fake-translate-retry");
    let repo = handoff_repo_fixture(&root);
    let store = BookPipelineStore::for_test(&root);
    let job_id = fake_handoff_ready_job(&store, &repo);
    let handed_off = store
        .load()
        .unwrap()
        .jobs
        .into_iter()
        .find(|job| job.id == job_id)
        .unwrap();
    let project_root = child_project_root(&handed_off);
    fs::write(
        project_root.join("source").join("source.md"),
        "# Alpha\n\nFirst body.\n\n# Beta\n\nSecond body.\n",
    )
    .unwrap();
    let executor = TranslationEngineFixtureExecutor::failing_once("chapter_002");

    advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();

    // One transient unit failure is the runner's own problem now: the stage
    // fails, schedules itself and comes back without anyone pressing retry.
    let recovered = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();

    // The automatic attempt inherits the existing retry-scope trimming, so
    // the unit that already translated is not paid for twice.
    assert_eq!(
        executor.requested_units(),
        vec![
            vec!["chapter_001".to_string(), "chapter_002".to_string()],
            vec!["chapter_002".to_string()]
        ]
    );
    assert_eq!(
        child_stage_status(&recovered, "translate"),
        STATUS_COMPLETED
    );
    assert_eq!(child_stage_status(&recovered, "expert_qa"), STATUS_PENDING);
    assert_eq!(recovered.current_stage_id, "expert_qa");
    assert!(recovered.last_error.is_none());
    let translate = recovered.children[0]
        .stages
        .iter()
        .find(|stage| stage.stage_id == "translate")
        .unwrap();
    let summary = translate.unit_summary.as_ref().unwrap();
    assert_eq!(
        (summary.total, summary.completed, summary.failed),
        (2, 2, 0)
    );
    assert_eq!(translate.attempt, 2);
    assert!(!translate
        .input_hashes
        .keys()
        .any(|key| key.starts_with("failedUnit:")));
    assert_eq!(
        recovered.children[0]
            .artifacts
            .iter()
            .filter(|artifact| artifact.kind == "chapter_translation")
            .count(),
        2
    );
    assert!(!recovered.children[0]
        .artifacts
        .iter()
        .any(|artifact| artifact.kind == "chapter_translation_degraded"));
    assert!(recovered.children[0]
        .artifacts
        .iter()
        .filter(|artifact| artifact.kind == "chapter_translation")
        .all(|artifact| artifact.sha256.is_some()
            && artifact.producer_stage.as_deref() == Some("translate")));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn expert_qa_retries_only_failed_unit_and_separates_fix_from_pass_attempt() {
    let root = temp_root("expert-qa-retry");
    let repo = handoff_repo_fixture(&root);
    let store = BookPipelineStore::for_test(&root);
    let job_id = fake_handoff_ready_job(&store, &repo);
    let handed_off = store
        .load()
        .unwrap()
        .jobs
        .into_iter()
        .find(|job| job.id == job_id)
        .unwrap();
    let project_root = child_project_root(&handed_off);
    fs::write(
        project_root.join("source").join("source.md"),
        "# Alpha\n\nFirst body.\n\n# Beta\n\nSecond body.\n",
    )
    .unwrap();
    let executor = TranslationEngineFixtureExecutor::succeeding();

    advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    let translated = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    let chapter_two = child_project_root(&translated)
        .join("chapters")
        .join("translated")
        .join("chapter_002.md");
    fs::write(&chapter_two, "# Broken\n").unwrap();

    let failed = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    assert_eq!(child_stage_status(&failed, "expert_qa"), STATUS_FAILED);
    assert!(failed.children[0].artifacts.iter().any(|artifact| {
        matches!(
            artifact.producer.stage_id.as_str(),
            "translate" | "expert_qa"
        )
    }));
    assert!(failed.children[0]
        .stages
        .iter()
        .find(|stage| stage.stage_id == "expert_qa")
        .unwrap()
        .unit_summary
        .is_some());
    let qa_stage = failed.children[0]
        .stages
        .iter()
        .find(|stage| stage.stage_id == "expert_qa")
        .unwrap();
    assert!(qa_stage.input_hashes.contains_key("failedUnit:chapter_002"));
    assert!(!qa_stage.input_hashes.contains_key("failedUnit:chapter_001"));
    assert_eq!(
        chapter_control(&failed, "chapter_001")["automationAttempt"],
        1
    );
    assert_eq!(
        chapter_control(&failed, "chapter_002")["automationAttempt"],
        1
    );

    let source_two = fs::read_to_string(
        child_project_root(&failed)
            .join("chapters")
            .join("src")
            .join("chapter_002.md"),
    )
    .unwrap();
    fs::write(
        &chapter_two,
        fixture_translation(&source_two, "chapter_002"),
    )
    .unwrap();
    let waiting = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();

    assert_eq!(child_stage_status(&waiting, "expert_qa"), STATUS_BLOCKED);
    assert_eq!(
        chapter_control(&waiting, "chapter_001")["automationAttempt"],
        1
    );
    assert_eq!(
        chapter_control(&waiting, "chapter_002")["automationAttempt"],
        2
    );
    satisfy_qa_handoff(&waiting);
    let ready = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();

    assert_eq!(child_stage_status(&ready, "expert_qa"), STATUS_COMPLETED);
    assert_eq!(
        child_stage_status(&ready, "approve_promotion"),
        STATUS_READY
    );
    let control = chapter_control(&ready, "chapter_002");
    assert_eq!(control["fixAttempt"], 2);
    assert_eq!(control["closureEvidence"]["passAttempt"], 3);
    assert_ne!(
        control["fixAttempt"],
        control["closureEvidence"]["passAttempt"]
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn fast_qa_expands_a_defective_sample_until_the_next_clean_unit() {
    let root = temp_root("expert-qa-expansion");
    let repo = handoff_repo_fixture(&root);
    let store = BookPipelineStore::for_test(&root);
    let job_id = fake_handoff_ready_job(&store, &repo);
    let handed_off = store
        .load()
        .unwrap()
        .jobs
        .into_iter()
        .find(|job| job.id == job_id)
        .unwrap();
    fs::write(
        child_project_root(&handed_off)
            .join("source")
            .join("source.md"),
        "# One\n\nBody one.\n\n# Two\n\nBody two.\n\n# Three\n\nBody three.\n",
    )
    .unwrap();
    let executor = TranslationEngineFixtureExecutor::succeeding();

    advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    let initial_waiting =
        advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    let initial_handoff = qa_handoff(&initial_waiting);
    assert_eq!(initial_handoff.base_unit_ids.len(), 2);
    assert!(initial_handoff.expansion_unit_ids.is_empty());
    let defective = initial_handoff.base_unit_ids[0].clone();
    set_expert_review(&initial_waiting, &defective, "failed", 1);
    for unit_id in initial_handoff.base_unit_ids.iter().skip(1) {
        set_expert_review(&initial_waiting, unit_id, "pass", 0);
    }

    let expanded = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    let expanded_handoff = qa_handoff(&expanded);
    assert_eq!(expanded_handoff.expansion_unit_ids.len(), 1);
    let expansion_unit = expanded_handoff.expansion_unit_ids[0].clone();
    set_expert_review(&expanded, &expansion_unit, "pass", 0);

    let clean_boundary =
        advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    assert_eq!(
        child_stage_status(&clean_boundary, "expert_qa"),
        STATUS_BLOCKED
    );
    assert_eq!(qa_handoff(&clean_boundary).expansion_unit_ids.len(), 1);
    set_expert_review(&clean_boundary, &defective, "pass", 0);

    let ready = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    assert_eq!(child_stage_status(&ready, "expert_qa"), STATUS_COMPLETED);
    assert_eq!(
        child_stage_status(&ready, "approve_promotion"),
        STATUS_READY
    );
    assert_eq!(qa_handoff(&ready).expansion_unit_ids, vec![expansion_unit]);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn translation_and_control_hash_changes_invalidate_promotion_approval() {
    let root = temp_root("promotion-hash-invalidation");
    let repo = handoff_repo_fixture(&root);
    let store = BookPipelineStore::for_test(&root);
    let job_id = fake_handoff_ready_job(&store, &repo);
    let executor = TranslationEngineFixtureExecutor::succeeding();

    advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    let waiting = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    satisfy_qa_handoff(&waiting);
    let ready = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    let first_approval = approve_ready_promotion_for_test(&store, &job_id);

    let translation_path = child_project_root(&ready)
        .join("chapters")
        .join("translated")
        .join("chapter_001.md");
    let translation = fs::read_to_string(&translation_path).unwrap();
    fs::write(
        &translation_path,
        translation.replace("Translated:", "Revised:"),
    )
    .unwrap();
    let translation_invalidated =
        advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();

    assert_eq!(
        child_stage_status(&translation_invalidated, "expert_qa"),
        STATUS_BLOCKED
    );
    assert_eq!(
        child_stage_status(&translation_invalidated, "approve_promotion"),
        STATUS_PENDING
    );
    assert!(!translation_invalidated
        .approval_references
        .iter()
        .any(|approval| approval.approval_id == first_approval));

    satisfy_qa_handoff(&translation_invalidated);
    let rereadied = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    assert_eq!(
        child_stage_status(&rereadied, "approve_promotion"),
        STATUS_READY
    );
    let second_approval = approve_ready_promotion_for_test(&store, &job_id);
    let control_path = child_project_root(&rereadied)
        .join("qa")
        .join("chapter_controls")
        .join("chapter_001.json");
    let mut control: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&control_path).unwrap()).unwrap();
    control["externalNote"] = serde_json::json!("changed after approval");
    fs::write(
        &control_path,
        serde_json::to_string_pretty(&control).unwrap() + "\n",
    )
    .unwrap();

    let control_invalidated =
        advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    assert_eq!(
        child_stage_status(&control_invalidated, "approve_promotion"),
        STATUS_READY
    );
    assert!(!control_invalidated
        .approval_references
        .iter()
        .any(|approval| approval.approval_id == second_approval));
    assert!(control_invalidated.children[0]
        .stages
        .iter()
        .find(|stage| stage.stage_id == "approve_promotion")
        .unwrap()
        .approval_id
        .is_none());
    let _ = fs::remove_dir_all(root);
}

/// Drive the fake pipeline to a completed reading build so there is a real
/// EPUB artifact, with a real digest, to record evidence against.
fn completed_reading_job(
    store: &BookPipelineStore,
    repo: &Path,
    executor: &ReadingPipelineFixtureExecutor,
) -> (String, BookPipelineJob) {
    let (job_id, waiting) = fake_job_waiting_for_expert_qa(store, repo, executor);
    satisfy_qa_handoff(&waiting);
    advance_job_with_executor(store, &job_id, None, false, executor).unwrap();
    let ready = store
        .load()
        .unwrap()
        .jobs
        .into_iter()
        .find(|job| job.id == job_id)
        .unwrap();
    approve_job_gate(
        store,
        &job_id,
        Some(&ready.children[0].id),
        "approve_promotion",
        true,
    )
    .unwrap();
    // promote, build_reading, validate_reading
    for _ in 0..3 {
        advance_job_with_executor(store, &job_id, None, false, executor).unwrap();
    }
    let completed = store
        .load()
        .unwrap()
        .jobs
        .into_iter()
        .find(|job| job.id == job_id)
        .unwrap();
    (job_id, completed)
}

#[test]
fn reading_validation_status_marks_every_finished_stage_explicitly() {
    let root = temp_root("reading-validation-status");
    fs::create_dir_all(root.join("qa")).unwrap();
    fs::write(
        root.join("qa/status.md"),
        "# QA Status\n\n- extraction: completed\n- split: pending rerun\n- translation: pending rerun\n- expert QA: pending\n- reading output: pending\n- EPUBCheck: pending\n",
    )
    .unwrap();

    write_reading_validation_status(&root, &EpubCheckSummary::default(), true, &[]).unwrap();

    let status = fs::read_to_string(root.join("qa/status.md")).unwrap();
    for completed_line in [
        "- split: passed",
        "- translation: passed",
        "- expert QA: passed",
        "- reading output: passed",
        "- EPUBCheck: passed",
    ] {
        assert!(status.contains(completed_line), "{status}");
    }
    assert!(!status.contains(": pending"), "{status}");
    assert!(status.contains("- EPUBCheck: fatal=0, error=0, warning=0"));
    let _ = fs::remove_dir_all(root);
}

// Story 18's second half — "and a real reader" — had nowhere to land, so the
// only place to note it was qa/status.md, which validate_reading rewrites.
#[test]
fn reader_evidence_survives_revalidation_and_reaches_qa_status() {
    let root = temp_root("reader-evidence-persists");
    let repo = handoff_repo_fixture(&root);
    let store = BookPipelineStore::for_test(&root);
    let executor = ReadingPipelineFixtureExecutor::passing();
    let (job_id, completed) = completed_reading_job(&store, &repo, &executor);
    assert_eq!(
        child_stage_status(&completed, "validate_reading"),
        STATUS_COMPLETED
    );
    let project_root = child_project_root(&completed);
    assert!(fs::read_to_string(project_root.join("qa/status.md"))
        .unwrap()
        .contains("- reader verification: not recorded"));

    let recorded = record_reader_evidence(
        &store,
        &job_id,
        Some(&completed.children[0].id),
        "reading_epub",
        "Apple Books",
        "7.2",
        "passed",
    )
    .unwrap();

    let evidence = &recorded.children[0].reader_evidence;
    assert_eq!(evidence.len(), 1);
    assert_eq!(evidence[0].reader, "Apple Books");
    assert_eq!(evidence[0].conclusion, "passed");
    assert!(!evidence[0].stale);
    let epub_sha256 = recorded.children[0]
        .artifacts
        .iter()
        .find(|artifact| artifact.kind == "reading_epub")
        .and_then(|artifact| artifact.sha256.clone())
        .unwrap();
    assert_eq!(evidence[0].artifact_sha256, epub_sha256);

    // The record carries the artifact's identity, never its location.
    let payload = serde_json::to_string(&evidence[0]).unwrap();
    assert!(!payload.contains(&display_path(&project_root)), "{payload}");
    assert!(!payload.contains('/'), "{payload}");

    // Re-running validate_reading must not quietly erase it, and the report
    // it regenerates now says the same thing the job state does.
    run_validate_reading_stage(&recorded, &recorded.children[0], &executor).unwrap();
    let revalidated = store
        .load()
        .unwrap()
        .jobs
        .into_iter()
        .find(|job| job.id == job_id)
        .unwrap();

    assert_eq!(revalidated.children[0].reader_evidence, *evidence);
    let qa_status = fs::read_to_string(project_root.join("qa/status.md")).unwrap();
    assert!(
        qa_status.contains("- reader verification: Apple Books 7.2 on reading_epub — passed"),
        "{qa_status}"
    );
    assert!(qa_status.contains(&epub_sha256), "{qa_status}");

    let _ = fs::remove_dir_all(root);
}

// One reading session must not vouch for every later build of the book.
#[test]
fn rebuilding_the_epub_makes_reader_evidence_stale() {
    let root = temp_root("reader-evidence-stale");
    let repo = handoff_repo_fixture(&root);
    let store = BookPipelineStore::for_test(&root);
    let executor = ReadingPipelineFixtureExecutor::passing();
    let (job_id, completed) = completed_reading_job(&store, &repo, &executor);
    let child_id = completed.children[0].id.clone();
    let recorded = record_reader_evidence(
        &store,
        &job_id,
        Some(&child_id),
        "reading_epub",
        "Calibre",
        "8.4",
        "passed",
    )
    .unwrap();
    assert!(!recorded.children[0].reader_evidence[0].stale);

    // Rebuild the EPUB: same artifact, different bytes. The evidence still
    // describes what someone read, but no longer describes what is built.
    let mut state = store.load().unwrap();
    let job_index = find_job_index(&state, &job_id).unwrap();
    let artifact = state.jobs[job_index].children[0]
        .artifacts
        .iter_mut()
        .find(|artifact| artifact.kind == "reading_epub")
        .unwrap();
    let epub_path = PathBuf::from(&artifact.path);
    fs::write(&epub_path, "rebuilt epub bytes").unwrap();
    let rebuilt_sha256 = sha256_file(&epub_path).unwrap();
    artifact.artifact_id = format!(
        "artifact-{}",
        sha256_str(&format!(
            "{}\0{}\0{}",
            artifact.kind, artifact.path, rebuilt_sha256
        ))
    );
    artifact.sha256 = Some(rebuilt_sha256);
    artifact.size_bytes = Some(fs::metadata(&epub_path).unwrap().len());
    derive_job(&mut state.jobs[job_index]);
    let rebuilt = state.jobs[job_index].clone();
    store.save(&state).unwrap();

    assert_eq!(rebuilt.children[0].reader_evidence.len(), 1);
    assert!(
        rebuilt.children[0].reader_evidence[0].stale,
        "evidence must not survive the artifact it was taken against"
    );
    assert_eq!(rebuilt.children[0].reader_evidence[0].reader, "Calibre");

    let _ = fs::remove_dir_all(root);
}

#[test]
fn reader_evidence_is_optional_and_validated() {
    let root = temp_root("reader-evidence-optional");
    let repo = handoff_repo_fixture(&root);
    let store = BookPipelineStore::for_test(&root);
    let executor = ReadingPipelineFixtureExecutor::passing();
    let (job_id, completed) = completed_reading_job(&store, &repo, &executor);
    let child_id = completed.children[0].id.clone();

    // Promotion already happened, and validation already completed, with no
    // reader evidence anywhere: recording it is not a precondition of either.
    assert_eq!(completed.status, STATUS_COMPLETED);
    assert!(completed.children[0].reader_evidence.is_empty());
    assert_eq!(
        child_stage_status(&completed, "approve_promotion"),
        STATUS_COMPLETED
    );

    for (kind, reader, version, conclusion) in [
        ("reading_markdown", "Apple Books", "7.2", "passed"),
        ("reading_epub", "", "7.2", "passed"),
        ("reading_epub", "Apple Books", "7.2", "looked fine to me"),
    ] {
        assert!(
            record_reader_evidence(
                &store,
                &job_id,
                Some(&child_id),
                kind,
                reader,
                version,
                conclusion,
            )
            .is_err(),
            "{kind}/{reader}/{conclusion} should be rejected"
        );
    }

    // Re-reading the same book in the same app supersedes rather than piles up.
    for conclusion in ["passed", "failed"] {
        record_reader_evidence(
            &store,
            &job_id,
            Some(&child_id),
            "reading_epub",
            "Thorium",
            "3.1",
            conclusion,
        )
        .unwrap();
    }
    let stored = store
        .load()
        .unwrap()
        .jobs
        .into_iter()
        .find(|job| job.id == job_id)
        .unwrap();
    assert_eq!(stored.children[0].reader_evidence.len(), 1);
    assert_eq!(stored.children[0].reader_evidence[0].conclusion, "failed");

    let _ = fs::remove_dir_all(root);
}

#[test]
fn fake_pipeline_promotes_builds_validates_and_completes() {
    let root = temp_root("fake-reading-complete");
    let repo = handoff_repo_fixture(&root);
    let store = BookPipelineStore::for_test(&root);
    let executor = ReadingPipelineFixtureExecutor::passing();
    let (job_id, waiting) = fake_job_waiting_for_expert_qa(&store, &repo, &executor);
    let project_root = child_project_root(&waiting);
    fs::write(
        project_root.join("chapters/translated/unapproved.md"),
        "# Unapproved\n",
    )
    .unwrap();
    satisfy_qa_handoff(&waiting);
    advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    let ready = store
        .load()
        .unwrap()
        .jobs
        .into_iter()
        .find(|job| job.id == job_id)
        .unwrap();
    let approved = approve_job_gate(
        &store,
        &job_id,
        Some(&ready.children[0].id),
        "approve_promotion",
        true,
    )
    .unwrap();
    assert_eq!(
        child_stage_status(&approved, "approve_promotion"),
        STATUS_COMPLETED
    );

    let promoted = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();

    assert_eq!(
        child_stage_status(&promoted, "approve_promotion"),
        STATUS_COMPLETED
    );
    assert_eq!(child_stage_status(&promoted, "promote"), STATUS_COMPLETED);
    assert!(!project_root.join("chapters/final/unapproved.md").exists());
    assert_eq!(
        promoted.children[0]
            .artifacts
            .iter()
            .filter(|artifact| artifact.kind == "chapter_final")
            .count(),
        1
    );
    assert!(promoted.children[0].artifacts.iter().any(|artifact| {
        artifact.kind == "promotion_manifest"
            && artifact.producer_stage.as_deref() == Some("promote")
    }));

    let built = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    assert_eq!(
        child_stage_status(&built, "build_reading"),
        STATUS_COMPLETED
    );
    let completed = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();

    assert_eq!(completed.status, STATUS_COMPLETED);
    assert_eq!(completed.output_formats, default_output_formats());
    assert!(!project_root
        .join("output/reading/book_bilingual.epub")
        .exists());
    assert_eq!(
        child_stage_status(&completed, "validate_reading"),
        STATUS_COMPLETED
    );
    assert_eq!(
        child_stage_status(&completed, "build_digest"),
        STATUS_SKIPPED
    );
    for (kind, stage) in [
        ("reading_markdown", "build_reading"),
        ("reading_html", "build_reading"),
        ("reading_epub", "build_reading"),
        ("epubcheck_report", "validate_reading"),
    ] {
        assert!(completed.children[0].artifacts.iter().any(|artifact| {
            artifact.kind == kind && artifact.producer_stage.as_deref() == Some(stage)
        }));
    }
    assert!(!completed.children[0]
        .artifacts
        .iter()
        .any(|artifact| artifact.kind == "reading_bilingual_epub"));
    let report: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(project_root.join("output/epubcheck.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(report["checker"]["nFatal"], 0);
    assert_eq!(report["checker"]["nError"], 0);
    let qa_status = fs::read_to_string(project_root.join("qa/status.md")).unwrap();
    for completed_line in [
        "- split: passed",
        "- translation: passed",
        "- expert QA: passed",
        "- reading output: passed",
        "- EPUBCheck: passed",
    ] {
        assert!(
            qa_status.contains(completed_line),
            "{completed_line} missing from:\n{qa_status}"
        );
    }
    assert!(!qa_status.contains(": pending"), "{qa_status}");
    assert!(qa_status.contains(
        "- accepted residual risks: 1 EPUBCheck warning(s), accepted for local reading output"
    ));
    assert_eq!(
        executor.command_labels(),
        vec![
            TRANSLATION_ENGINE_COMMAND_LABEL.to_string(),
            READING_BUILD_COMMAND_LABEL.to_string(),
            EPUBCHECK_COMMAND_LABEL.to_string(),
        ]
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn fake_pipeline_builds_and_validates_bilingual_epub_when_selected() {
    let root = temp_root("fake-bilingual-complete");
    let repo = handoff_repo_fixture(&root);
    let store = BookPipelineStore::for_test(&root);
    let executor = ReadingPipelineFixtureExecutor::passing();
    let mut output_formats = default_output_formats();
    output_formats.push(OUTPUT_FORMAT_BILINGUAL.into());
    let job_id = fake_handoff_ready_job_with_output_formats(
        &store,
        &repo,
        false,
        false,
        output_formats.clone(),
    );
    advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    let waiting = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    assert_eq!(child_stage_status(&waiting, "expert_qa"), STATUS_BLOCKED);
    let project_root = child_project_root(&waiting);
    satisfy_qa_handoff(&waiting);
    advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    approve_ready_promotion_for_test(&store, &job_id);
    advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();

    let built = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    assert_eq!(
        child_stage_status(&built, "build_reading"),
        STATUS_COMPLETED
    );
    assert!(project_root
        .join("output/reading/book_bilingual.epub")
        .is_file());
    assert!(built.children[0].artifacts.iter().any(|artifact| {
        artifact.kind == "reading_bilingual_epub"
            && artifact.producer_stage.as_deref() == Some("build_reading")
            && artifact.sha256.is_some()
    }));

    let completed = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    assert_eq!(completed.status, STATUS_COMPLETED);
    assert_eq!(completed.output_formats, output_formats);
    assert!(completed.children[0].artifacts.iter().any(|artifact| {
        artifact.kind == "bilingual_epubcheck_report"
            && artifact.producer_stage.as_deref() == Some("validate_reading")
            && artifact.sha256.is_some()
    }));
    assert!(project_root
        .join("output/epubcheck_bilingual.json")
        .is_file());
    let qa_status = fs::read_to_string(project_root.join("qa/status.md")).unwrap();
    assert!(qa_status.contains("- reading output: passed"));
    assert!(qa_status.contains(
        "- accepted residual risks: 2 EPUBCheck warning(s), accepted for local reading output"
    ));
    assert!(completed
        .log_summary
        .iter()
        .any(|line| line.contains("alignment=paragraph")));
    assert_eq!(
        executor.command_labels(),
        vec![
            TRANSLATION_ENGINE_COMMAND_LABEL.to_string(),
            READING_BUILD_COMMAND_LABEL.to_string(),
            BILINGUAL_BUILD_COMMAND_LABEL.to_string(),
            EPUBCHECK_COMMAND_LABEL.to_string(),
            EPUBCHECK_COMMAND_LABEL.to_string(),
        ]
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn fake_bilingual_pipeline_logs_whole_chapter_fallback_for_mismatched_paragraphs() {
    let root = temp_root("fake-bilingual-fallback");
    let repo = handoff_repo_fixture(&root);
    let store = BookPipelineStore::for_test(&root);
    let executor = ReadingPipelineFixtureExecutor::passing_with_bilingual_fallback();
    let mut output_formats = default_output_formats();
    output_formats.push(OUTPUT_FORMAT_BILINGUAL.into());
    let job_id =
        fake_handoff_ready_job_with_output_formats(&store, &repo, false, false, output_formats);
    advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    let waiting = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    satisfy_qa_handoff(&waiting);
    advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    approve_ready_promotion_for_test(&store, &job_id);
    advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();

    let built = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();

    assert_eq!(
        child_stage_status(&built, "build_reading"),
        STATUS_COMPLETED
    );
    assert!(built
        .log_summary
        .iter()
        .any(|line| line.contains("alignment=chapter-fallback")));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn fake_pipeline_builds_digest_when_book_intent_is_enabled() {
    let root = temp_root("fake-digest-complete");
    let repo = handoff_repo_fixture(&root);
    let store = BookPipelineStore::for_test(&root);
    let executor = ReadingPipelineFixtureExecutor::passing_with_digest();
    let (job_id, waiting) =
        fake_job_waiting_for_expert_qa_with_digest(&store, &repo, &executor, true);
    let project_root = child_project_root(&waiting);
    fs::write(
        project_root.join("digest.config.json"),
        "{\n  \"max_section_chars\": 2400\n}\n",
    )
    .unwrap();
    fs::write(
        project_root.join("metadata/book.yaml"),
        "title: Digest Fixture Title\nlanguage: zh-CN\n",
    )
    .unwrap();
    satisfy_qa_handoff(&waiting);
    advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    approve_ready_promotion_for_test(&store, &job_id);
    advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    let validated = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();

    assert_eq!(
        child_stage_status(&validated, "validate_reading"),
        STATUS_COMPLETED
    );
    assert_eq!(
        child_stage_status(&validated, "build_digest"),
        STATUS_PENDING
    );
    let completed = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();

    assert_eq!(completed.status, STATUS_COMPLETED);
    assert_eq!(
        child_stage_status(&completed, "build_digest"),
        STATUS_COMPLETED
    );
    for kind in [
        "digest_epub",
        "digest_xhtml",
        "digest_knowledge_map",
        "digest_review_checklist",
        "digest_report",
        "digest_epubcheck_report",
    ] {
        assert!(completed.children[0].artifacts.iter().any(|artifact| {
            artifact.kind == kind
                && artifact.producer_stage.as_deref() == Some("build_digest")
                && artifact.sha256.is_some()
        }));
    }
    let digest_stage = completed.children[0]
        .stages
        .iter()
        .find(|stage| stage.stage_id == "build_digest")
        .unwrap();
    for key in [
        "readingEpubSha256",
        "epubcheckReportSha256",
        "sourceManifestSha256",
        "bookMetadataSha256",
        "digestConfigSha256",
    ] {
        assert!(
            digest_stage.input_hashes.contains_key(key),
            "missing key {key}"
        );
    }
    let report: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(project_root.join("qa/digest/digest_report.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(report["status"], "PASS");
    assert_eq!(report["merged"], true);
    assert_eq!(report["source_epub"], "output/reading/book.epub");
    assert_eq!(report["output_epub"], "output/reading/book_digest.epub");
    let epubcheck_report: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(project_root.join("output/digest_epubcheck.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(epubcheck_report["checker"]["nFatal"], 0);
    assert_eq!(epubcheck_report["checker"]["nError"], 0);
    let config: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(project_root.join("digest.config.json")).unwrap())
            .unwrap();
    assert_eq!(config["max_section_chars"], 2400);
    assert_eq!(
        executor.command_labels(),
        vec![
            TRANSLATION_ENGINE_COMMAND_LABEL.to_string(),
            READING_BUILD_COMMAND_LABEL.to_string(),
            EPUBCHECK_COMMAND_LABEL.to_string(),
            DIGEST_BUILD_COMMAND_LABEL.to_string(),
            EPUBCHECK_COMMAND_LABEL.to_string(),
        ]
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn fake_expert_pipeline_builds_digest_when_book_intent_is_enabled() {
    let root = temp_root("fake-expert-digest-complete");
    let repo = handoff_repo_fixture(&root);
    let store = BookPipelineStore::for_test(&root);
    let executor = ReadingPipelineFixtureExecutor::passing_with_digest();
    let job_id = fake_handoff_ready_job_with_digest(&store, &repo);
    let handed_off = store
        .load()
        .unwrap()
        .jobs
        .into_iter()
        .find(|job| job.id == job_id)
        .unwrap();
    fs::write(
        child_project_root(&handed_off).join("metadata/book.yaml"),
        "title: Digest Fixture Title\nlanguage: zh-CN\n",
    )
    .unwrap();
    configure_expert_job(&store, &job_id);

    advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    approve_ready_translation_for_test(&store, &job_id);
    let translation_waiting =
        advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    satisfy_translation_handoff(&translation_waiting);
    advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    let qa_waiting = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    satisfy_qa_handoff(&qa_waiting);
    advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    approve_ready_promotion_for_test(&store, &job_id);
    advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    let completed = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();

    assert_eq!(completed.translation_mode, TRANSLATION_MODE_EXPERT);
    assert_eq!(completed.status, STATUS_COMPLETED);
    assert_eq!(
        child_stage_status(&completed, "build_digest"),
        STATUS_COMPLETED
    );
    assert!(completed.children[0].artifacts.iter().any(|artifact| {
        artifact.kind == "digest_epub"
            && artifact.producer_stage.as_deref() == Some("build_digest")
            && artifact.sha256.is_some()
    }));
    assert_eq!(
        executor.command_labels(),
        vec![
            READING_BUILD_COMMAND_LABEL.to_string(),
            EPUBCHECK_COMMAND_LABEL.to_string(),
            DIGEST_BUILD_COMMAND_LABEL.to_string(),
            EPUBCHECK_COMMAND_LABEL.to_string(),
        ]
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn digest_epubcheck_failure_marks_build_failed_and_can_retry() {
    let root = temp_root("fake-digest-epubcheck-failed");
    let repo = handoff_repo_fixture(&root);
    let store = BookPipelineStore::for_test(&root);
    let executor = ReadingPipelineFixtureExecutor::failing_digest_epubcheck();
    let (job_id, waiting) =
        fake_job_waiting_for_expert_qa_with_digest(&store, &repo, &executor, true);
    let project_root = child_project_root(&waiting);
    fs::write(
        project_root.join("metadata/book.yaml"),
        "title: Digest Fixture Title\nlanguage: zh-CN\n",
    )
    .unwrap();
    satisfy_qa_handoff(&waiting);
    advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    approve_ready_promotion_for_test(&store, &job_id);
    advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();

    let failed = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();

    assert_eq!(failed.status, STATUS_FAILED);
    assert_eq!(failed.current_stage_id, "build_digest");
    assert_eq!(failed.current_step, "build_digest stage failed");
    assert_eq!(child_stage_status(&failed, "build_digest"), STATUS_FAILED);
    assert!(failed.last_error.as_deref().is_some_and(
        |error| error.contains("EPUBCheck reported 0 fatal finding(s) and 1 error(s)")
    ));
    assert!(failed.children[0].artifacts.iter().any(|artifact| {
        artifact.kind == "digest_epubcheck_report"
            && artifact.producer_stage.as_deref() == Some("build_digest")
    }));
    let failed_report: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(project_root.join("output/digest_epubcheck.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(failed_report["checker"]["nError"], 1);

    let retry_executor = ReadingPipelineFixtureExecutor::passing_with_digest();
    let completed =
        advance_job_with_executor(&store, &job_id, None, false, &retry_executor).unwrap();

    assert_eq!(completed.status, STATUS_COMPLETED);
    assert_eq!(
        child_stage_status(&completed, "build_digest"),
        STATUS_COMPLETED
    );
    // Three automatic attempts exhausted the stage's budget before the
    // operator's own retry, which is the fourth.
    assert_eq!(
        completed.children[0]
            .stages
            .iter()
            .find(|stage| stage.stage_id == "build_digest")
            .unwrap()
            .attempt,
        DEFAULT_STAGE_MAX_ATTEMPTS + 1
    );
    let passed_report: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(project_root.join("output/digest_epubcheck.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(passed_report["checker"]["nError"], 0);
    assert_eq!(
        retry_executor.command_labels(),
        vec![
            DIGEST_BUILD_COMMAND_LABEL.to_string(),
            EPUBCHECK_COMMAND_LABEL.to_string(),
        ]
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn epubcheck_failure_stops_at_validation_failed() {
    let root = temp_root("fake-reading-validation-failed");
    let repo = handoff_repo_fixture(&root);
    let store = BookPipelineStore::for_test(&root);
    let executor = ReadingPipelineFixtureExecutor::failing_epubcheck();
    let (job_id, waiting) = fake_job_waiting_for_expert_qa(&store, &repo, &executor);
    satisfy_qa_handoff(&waiting);
    advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    approve_ready_promotion_for_test(&store, &job_id);
    advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();

    let failed = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();

    assert_eq!(failed.status, STATUS_FAILED);
    assert_eq!(failed.current_stage_id, "validate_reading");
    assert_eq!(failed.current_step, "validation_failed");
    assert_eq!(
        child_stage_status(&failed, "validate_reading"),
        STATUS_FAILED
    );
    assert_ne!(failed.status, STATUS_COMPLETED);
    assert!(failed.children[0]
        .artifacts
        .iter()
        .any(|artifact| artifact.kind == "epubcheck_report"));
    let qa_status = fs::read_to_string(child_project_root(&failed).join("qa/status.md")).unwrap();
    assert!(qa_status.contains("- reading output: failed"));
    assert!(qa_status.contains("fatal=0, error=1, warning=0"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn real_markdown_job_stops_at_promotion_gate() {
    let root = temp_root("real-promotion-gate");
    let repo = handoff_repo_fixture(&root);
    let source_path = root.join("source.md");
    let store = BookPipelineStore::for_test(&root);
    let job_id = handoff_ready_child_job(
        &store,
        &repo,
        &source_path,
        "# Real chapter\n\nBody paragraph.\n",
    );
    let executor = TranslationEngineFixtureExecutor::succeeding();
    advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    approve_ready_translation_for_test(&store, &job_id);
    advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    let waiting = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    satisfy_qa_handoff(&waiting);

    let ready = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    let stopped = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();

    assert_eq!(
        child_stage_status(&ready, "approve_promotion"),
        STATUS_READY
    );
    assert_eq!(
        child_stage_status(&stopped, "approve_promotion"),
        STATUS_READY
    );
    assert_eq!(child_stage_status(&stopped, "promote"), STATUS_PENDING);
    assert_eq!(stopped.current_stage_id, "approve_promotion");
    assert!(!stopped.approval_references.iter().any(|approval| {
        approval.child_job_id == stopped.children[0].id && approval.stage_id == "approve_promotion"
    }));
    assert!(
        fs::read_dir(child_project_root(&stopped).join("chapters/final"))
            .unwrap()
            .next()
            .is_none()
    );
    let _ = fs::remove_dir_all(root);
}

fn stage_attempt(job: &BookPipelineJob, stage_id: &str) -> u32 {
    job.children[0]
        .stages
        .iter()
        .find(|stage| stage.stage_id == stage_id)
        .unwrap()
        .attempt
}

fn artifact_sha(job: &BookPipelineJob, kind: &str) -> Option<String> {
    job.children[0]
        .artifacts
        .iter()
        .find(|artifact| artifact.kind == kind)
        .unwrap()
        .sha256
        .clone()
}

fn chapter_source_count(job: &BookPipelineJob) -> usize {
    job.children[0]
        .artifacts
        .iter()
        .filter(|artifact| artifact.kind == "chapter_source")
        .count()
}

fn child_source_md(job: &BookPipelineJob) -> PathBuf {
    child_project_root(job).join("source").join("source.md")
}

#[test]
fn advance_reuses_completed_stages_without_rerunning() {
    let root = temp_root("advance-idempotent");
    let repo = handoff_repo_fixture(&root);
    let source_path = root.join("source.md");
    let store = BookPipelineStore::for_test(&root);
    let job_id = handoff_ready_child_job(
        &store,
        &repo,
        &source_path,
        "# One\n\nBody one.\n\n# Two\n\nBody two.\n",
    );

    advance_job(&store, &job_id, None, false).unwrap();
    let prepared = advance_job(&store, &job_id, None, false).unwrap();
    let split_attempt = stage_attempt(&prepared, "split");
    let prepare_attempt = stage_attempt(&prepared, "prepare");
    let source_map_sha = artifact_sha(&prepared, "source_map");

    let again = advance_job(&store, &job_id, None, false).unwrap();

    assert_eq!(child_stage_status(&again, "prepare"), STATUS_COMPLETED);
    assert_eq!(
        child_stage_status(&again, "approve_translation"),
        STATUS_READY
    );
    assert_eq!(stage_attempt(&again, "split"), split_attempt);
    assert_eq!(stage_attempt(&again, "prepare"), prepare_attempt);
    assert_eq!(artifact_sha(&again, "source_map"), source_map_sha);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn source_change_before_prepare_reruns_split_without_blocking() {
    let root = temp_root("advance-resplit");
    let repo = handoff_repo_fixture(&root);
    let source_path = root.join("source.md");
    let store = BookPipelineStore::for_test(&root);
    let job_id = handoff_ready_child_job(&store, &repo, &source_path, "# Only\n\nOriginal body.\n");

    let split_once = advance_job(&store, &job_id, None, false).unwrap();
    assert_eq!(chapter_source_count(&split_once), 1);
    let first_map = artifact_sha(&split_once, "source_map");

    fs::write(
        child_source_md(&split_once),
        "# First\n\nNew body.\n\n# Second\n\nMore body.\n",
    )
    .unwrap();
    let resplit = advance_job(&store, &job_id, None, false).unwrap();

    assert_eq!(child_stage_status(&resplit, "split"), STATUS_COMPLETED);
    assert_eq!(stage_attempt(&resplit, "split"), 2);
    assert_eq!(chapter_source_count(&resplit), 2);
    assert_ne!(artifact_sha(&resplit, "source_map"), first_map);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn split_bounds_an_oversized_book_unit_at_deeper_headings_without_losing_text() {
    let root = temp_root("advance-bounded-split");
    let repo = handoff_repo_fixture(&root);
    let source_path = root.join("source.md");
    let store = BookPipelineStore::for_test(&root);
    let section = "x".repeat(40 * 1024);
    let source = format!(
        "# Book\n\n## Page 1\n\n{section}\n\n## Page 2\n\n{section}\n\n## Page 3\n\n{section}\n"
    );
    let job_id = handoff_ready_child_job(&store, &repo, &source_path, &source);

    let split = advance_job(&store, &job_id, None, false).unwrap();
    let chapter_paths = split.children[0]
        .artifacts
        .iter()
        .filter(|artifact| artifact.kind == "chapter_source")
        .map(|artifact| PathBuf::from(&artifact.path))
        .collect::<Vec<_>>();
    let chapters = chapter_paths
        .iter()
        .map(|path| fs::read_to_string(path).unwrap())
        .collect::<Vec<_>>();

    assert_eq!(chapters.len(), 3);
    assert!(chapters.iter().all(|chapter| chapter.len() <= 64 * 1024));
    assert_eq!(chapters.concat(), source);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn split_bounds_an_oversized_heading_free_unit_at_paragraphs_without_losing_text() {
    let root = temp_root("advance-bounded-paragraph-split");
    let repo = handoff_repo_fixture(&root);
    let source_path = root.join("source.md");
    let store = BookPipelineStore::for_test(&root);
    let paragraph = "x".repeat(40 * 1024);
    let source = format!("{paragraph}\n\n{paragraph}\n\n{paragraph}\n");
    let job_id = handoff_ready_child_job(&store, &repo, &source_path, &source);

    let split = advance_job(&store, &job_id, None, false).unwrap();
    let chapters = split.children[0]
        .artifacts
        .iter()
        .filter(|artifact| artifact.kind == "chapter_source")
        .map(|artifact| fs::read_to_string(&artifact.path).unwrap())
        .collect::<Vec<_>>();

    assert_eq!(chapters.len(), 3);
    assert!(chapters.iter().all(|chapter| chapter.len() <= 64 * 1024));
    assert_eq!(chapters.concat(), source);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn split_hard_bounds_one_unbroken_paragraph_without_losing_text() {
    let root = temp_root("advance-hard-bounded-paragraph");
    let repo = handoff_repo_fixture(&root);
    let source_path = root.join("source.md");
    let store = BookPipelineStore::for_test(&root);
    let source = format!("{}\n", "界".repeat(50 * 1024));
    let job_id = handoff_ready_child_job(&store, &repo, &source_path, &source);

    let split = advance_job(&store, &job_id, None, false).unwrap();
    let chapters = split.children[0]
        .artifacts
        .iter()
        .filter(|artifact| artifact.kind == "chapter_source")
        .map(|artifact| fs::read_to_string(&artifact.path).unwrap())
        .collect::<Vec<_>>();

    assert!(chapters.len() > 1);
    assert!(chapters.iter().all(|chapter| chapter.len() <= 64 * 1024));
    assert_eq!(chapters.concat(), source);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn paragraph_fallback_never_splits_inside_a_fenced_code_block() {
    let root = temp_root("advance-fenced-paragraph-split");
    let repo = handoff_repo_fixture(&root);
    let source_path = root.join("source.md");
    let store = BookPipelineStore::for_test(&root);
    let prose = "x".repeat(40 * 1024);
    let code = "y".repeat(20 * 1024);
    let source = format!("{prose}\n\n```text\n{code}\n\n{code}\n```\n\n{prose}\n");
    let job_id = handoff_ready_child_job(&store, &repo, &source_path, &source);

    let split = advance_job(&store, &job_id, None, false).unwrap();
    let chapters = split.children[0]
        .artifacts
        .iter()
        .filter(|artifact| artifact.kind == "chapter_source")
        .map(|artifact| fs::read_to_string(&artifact.path).unwrap())
        .collect::<Vec<_>>();

    assert!(chapters
        .iter()
        .all(|chapter| chapter.matches("```").count() % 2 == 0));
    assert_eq!(chapters.concat(), source);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn mixed_fence_markers_do_not_close_the_active_code_block() {
    let text = "```text\none\n~~~\n\ntwo\n```\ntail\n";

    let atoms = structural_text_atoms(text);

    assert_eq!(
        atoms,
        vec![("```text\none\n~~~\n\ntwo\n```\n", false), ("tail\n", true),]
    );
}

#[test]
fn hard_bound_separates_a_small_fence_from_oversized_plain_text() {
    let text = format!("```text\nsmall\n```\n{}", "界".repeat(40 * 1024));

    let pieces = hard_bound_text(&text);

    assert!(pieces.len() > 1);
    assert!(pieces
        .iter()
        .all(|piece| piece.len() <= MAX_TRANSLATION_UNIT_BYTES));
    assert_eq!(pieces.concat(), text);
    assert_eq!(pieces[0], "```text\nsmall\n```\n");
}

#[test]
fn source_change_after_prepare_blocks_split_pending_invalidation() {
    let root = temp_root("advance-block");
    let repo = handoff_repo_fixture(&root);
    let source_path = root.join("source.md");
    let store = BookPipelineStore::for_test(&root);
    let job_id = handoff_ready_child_job(
        &store,
        &repo,
        &source_path,
        "# One\n\nBody one.\n\n# Two\n\nBody two.\n",
    );

    advance_job(&store, &job_id, None, false).unwrap();
    let prepared = advance_job(&store, &job_id, None, false).unwrap();
    assert_eq!(child_stage_status(&prepared, "prepare"), STATUS_COMPLETED);

    fs::write(
        child_source_md(&prepared),
        "# Rewritten\n\nDifferent body.\n",
    )
    .unwrap();
    let blocked = advance_job(&store, &job_id, None, false).unwrap();

    assert_eq!(child_stage_status(&blocked, "split"), STATUS_BLOCKED);
    // Committed downstream work is rolled back to pending, never silently lost.
    assert_eq!(child_stage_status(&blocked, "prepare"), STATUS_PENDING);
    assert_eq!(
        child_stage_status(&blocked, "approve_translation"),
        STATUS_PENDING
    );
    assert!(blocked.children[0]
        .stages
        .iter()
        .find(|stage| stage.stage_id == "approve_translation")
        .unwrap()
        .approval_request
        .is_none());
    assert_eq!(blocked.children[0].status, STATUS_BLOCKED);
    assert_eq!(
        blocked.children[0].last_error.as_deref(),
        Some("source_changed_downstream_exists")
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn source_change_is_not_hidden_by_a_failed_downstream_stage() {
    let root = temp_root("advance-block-after-downstream-failure");
    let repo = handoff_repo_fixture(&root);
    let source_path = root.join("source.md");
    let store = BookPipelineStore::for_test(&root);
    let job_id = handoff_ready_child_job(
        &store,
        &repo,
        &source_path,
        "# One\n\nBody one.\n\n# Two\n\nBody two.\n",
    );

    advance_job(&store, &job_id, None, false).unwrap();
    let prepared = advance_job(&store, &job_id, None, false).unwrap();
    fs::write(
        child_source_md(&prepared),
        "# Rewritten\n\nDifferent body.\n",
    )
    .unwrap();
    let mut state = store.load().unwrap();
    let child = &mut state
        .jobs
        .iter_mut()
        .find(|job| job.id == job_id)
        .unwrap()
        .children[0];
    set_stage_status(
        child,
        "expert_qa",
        STATUS_FAILED,
        Some("simulated stale failure".into()),
    );

    let change = evaluate_split_freshness(child, false)
        .unwrap()
        .expect("changed source must supersede a stale downstream failure");

    assert!(matches!(change.action, SplitFreshnessAction::Block));
    assert!(change.stop_after);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn source_change_after_expert_qa_failure_persists_a_clean_invalidation_gate() {
    let root = temp_root("advance-block-after-expert-qa-failure");
    let repo = handoff_repo_fixture(&root);
    let store = BookPipelineStore::for_test(&root);
    let job_id = fake_handoff_ready_job(&store, &repo);
    let executor = TranslationEngineFixtureExecutor::succeeding();
    let handed_off = store
        .load()
        .unwrap()
        .jobs
        .into_iter()
        .find(|job| job.id == job_id)
        .unwrap();
    fs::write(
        child_source_md(&handed_off),
        "# Alpha\n\nFirst body.\n\n# Beta\n\nSecond body.\n",
    )
    .unwrap();

    advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    let translated = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    fs::write(
        child_project_root(&translated)
            .join("chapters")
            .join("translated")
            .join("chapter_002.md"),
        "# Broken\n",
    )
    .unwrap();
    let failed = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();
    assert_eq!(child_stage_status(&failed, "expert_qa"), STATUS_FAILED);
    fs::write(child_source_md(&failed), "# Rewritten\n\nDifferent body.\n").unwrap();

    let blocked = advance_job_with_executor(&store, &job_id, None, false, &executor).unwrap();

    assert_eq!(child_stage_status(&blocked, "split"), STATUS_BLOCKED);
    assert_eq!(child_stage_status(&blocked, "expert_qa"), STATUS_PENDING);
    assert!(blocked.children[0].artifacts.iter().all(|artifact| {
        ordered_stage_index(&artifact.producer.stage_id)
            .is_none_or(|order| order <= ordered_stage_index("split").unwrap())
    }));
    for stage in blocked.children[0].stages.iter().filter(|stage| {
        ordered_stage_index(&stage.stage_id)
            .is_some_and(|order| order > ordered_stage_index("split").unwrap())
    }) {
        assert!(
            stage.artifact_ids.is_empty(),
            "{} kept artifact ids",
            stage.stage_id
        );
        assert!(
            stage.unit_summary.is_none(),
            "{} kept unit summary",
            stage.stage_id
        );
        assert!(
            stage.safe_error.is_none(),
            "{} kept safe error",
            stage.stage_id
        );
        assert_eq!(stage.attempt, 0, "{} kept retry attempts", stage.stage_id);
        assert!(
            stage.give_up_reason.is_none(),
            "{} kept give-up reason",
            stage.stage_id
        );
        assert!(
            stage.next_retry_at.is_none(),
            "{} kept retry deadline",
            stage.stage_id
        );
    }
    assert_eq!(
        blocked.children[0].last_error.as_deref(),
        Some("source_changed_downstream_exists")
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn prepared_unit_scope_repair_removes_legacy_downstream_records() {
    let root = temp_root("prepared-unit-scope-repair");
    let repo = handoff_repo_fixture(&root);
    let source_path = root.join("source.md");
    let store = BookPipelineStore::for_test(&root);
    let job_id = handoff_ready_child_job(
        &store,
        &repo,
        &source_path,
        "# One\n\nBody one.\n\n# Two\n\nBody two.\n",
    );

    advance_job(&store, &job_id, None, false).unwrap();
    let mut prepared = advance_job(&store, &job_id, None, false).unwrap();
    let child = &mut prepared.children[0];
    child.artifacts.push(BookPipelineArtifact {
        kind: "chapter_translation".into(),
        path: "/tmp/chapter_999.md".into(),
        producer: BookPipelineArtifactProducer {
            stage_id: "translate".into(),
            unit_id: Some("chapter_999".into()),
            ..BookPipelineArtifactProducer::default()
        },
        producer_stage: Some("translate".into()),
        ..BookPipelineArtifact::default()
    });
    child.artifacts.push(BookPipelineArtifact {
        kind: "chapter_control".into(),
        path: "/tmp/chapter_999.json".into(),
        producer: BookPipelineArtifactProducer {
            stage_id: "expert_qa".into(),
            unit_id: Some("chapter_999".into()),
            ..BookPipelineArtifactProducer::default()
        },
        producer_stage: Some("expert_qa".into()),
        ..BookPipelineArtifact::default()
    });
    let expert = child
        .stages
        .iter_mut()
        .find(|stage| stage.stage_id == "expert_qa")
        .unwrap();
    expert.attempt = 27;
    expert.unit_summary = Some(BookPipelineUnitSummary {
        total: 23,
        failed: 22,
        ..BookPipelineUnitSummary::default()
    });
    expert.artifact_ids.push("legacy-control".into());

    assert!(reconcile_prepared_unit_scope(child));
    assert!(!child
        .artifacts
        .iter()
        .any(|artifact| { artifact.producer.unit_id.as_deref() == Some("chapter_999") }));
    let expert = child
        .stages
        .iter()
        .find(|stage| stage.stage_id == "expert_qa")
        .unwrap();
    assert_eq!(expert.attempt, 0);
    assert!(expert.unit_summary.is_none());
    assert!(expert.artifact_ids.is_empty());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn explicit_invalidation_reruns_split_and_prepare_from_new_source() {
    let root = temp_root("advance-invalidate");
    let repo = handoff_repo_fixture(&root);
    let source_path = root.join("source.md");
    let store = BookPipelineStore::for_test(&root);
    let job_id = handoff_ready_child_job(
        &store,
        &repo,
        &source_path,
        "# One\n\nBody one.\n\n# Two\n\nBody two.\n",
    );

    advance_job(&store, &job_id, None, false).unwrap();
    let prepared = advance_job(&store, &job_id, None, false).unwrap();
    fs::write(
        child_source_md(&prepared),
        "# Merged\n\nSingle chapter now.\n",
    )
    .unwrap();
    let blocked = advance_job(&store, &job_id, None, false).unwrap();
    assert_eq!(child_stage_status(&blocked, "split"), STATUS_BLOCKED);

    let unblocked = advance_job(&store, &job_id, None, true).unwrap();
    assert_eq!(child_stage_status(&unblocked, "split"), STATUS_COMPLETED);
    assert_eq!(chapter_source_count(&unblocked), 1);
    assert_eq!(child_stage_status(&unblocked, "prepare"), STATUS_PENDING);

    let reprepared = advance_job(&store, &job_id, None, false).unwrap();
    assert_eq!(child_stage_status(&reprepared, "prepare"), STATUS_COMPLETED);
    assert_eq!(
        reprepared.children[0]
            .artifacts
            .iter()
            .filter(|artifact| artifact.kind == "translation_task_manifest")
            .count(),
        1
    );
    assert!(!child_project_root(&reprepared)
        .join("chapters")
        .join("src")
        .join("chapter_002.md")
        .exists());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn split_policy_upgrade_automatically_invalidates_downstream_and_reruns() {
    let root = temp_root("advance-split-policy-upgrade");
    let repo = handoff_repo_fixture(&root);
    let source_path = root.join("source.md");
    let store = BookPipelineStore::for_test(&root);
    let job_id = handoff_ready_child_job(
        &store,
        &repo,
        &source_path,
        "# One\n\nBody one.\n\n# Two\n\nBody two.\n",
    );

    advance_job(&store, &job_id, None, false).unwrap();
    let prepared = advance_job(&store, &job_id, None, false).unwrap();
    assert_eq!(child_stage_status(&prepared, "prepare"), STATUS_COMPLETED);

    let mut state = store.load().unwrap();
    let child = &mut state
        .jobs
        .iter_mut()
        .find(|job| job.id == job_id)
        .unwrap()
        .children[0];
    child
        .stages
        .iter_mut()
        .find(|stage| stage.stage_id == "split")
        .unwrap()
        .input_hashes
        .insert("splitPolicyVersion".into(), "split-policy-obsolete".into());
    store.save(&state).unwrap();

    let rerun = advance_job(&store, &job_id, None, false).unwrap();

    let split = rerun.children[0]
        .stages
        .iter()
        .find(|stage| stage.stage_id == "split")
        .unwrap();
    assert_eq!(split.status, STATUS_COMPLETED);
    assert_eq!(
        split
            .input_hashes
            .get("splitPolicyVersion")
            .map(String::as_str),
        Some(SPLIT_POLICY_VERSION)
    );
    assert_eq!(child_stage_status(&rerun, "prepare"), STATUS_PENDING);
    assert_eq!(
        child_stage_status(&rerun, "approve_translation"),
        STATUS_PENDING
    );
    let source_map = fs::read_to_string(
        child_project_root(&rerun)
            .join("metadata")
            .join("source_map.json"),
    )
    .unwrap();
    assert!(source_map.contains(SPLIT_POLICY_VERSION));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn source_change_still_blocks_when_split_policy_also_changed() {
    let root = temp_root("advance-source-and-policy-change");
    let repo = handoff_repo_fixture(&root);
    let source_path = root.join("source.md");
    let store = BookPipelineStore::for_test(&root);
    let job_id = handoff_ready_child_job(
        &store,
        &repo,
        &source_path,
        "# One\n\nBody one.\n\n# Two\n\nBody two.\n",
    );

    advance_job(&store, &job_id, None, false).unwrap();
    let prepared = advance_job(&store, &job_id, None, false).unwrap();
    fs::write(
        child_source_md(&prepared),
        "# Rewritten\n\nDifferent body.\n",
    )
    .unwrap();
    let mut state = store.load().unwrap();
    state
        .jobs
        .iter_mut()
        .find(|job| job.id == job_id)
        .unwrap()
        .children[0]
        .stages
        .iter_mut()
        .find(|stage| stage.stage_id == "split")
        .unwrap()
        .input_hashes
        .insert("splitPolicyVersion".into(), "split-policy-obsolete".into());
    store.save(&state).unwrap();

    let blocked = advance_job(&store, &job_id, None, false).unwrap();

    assert_eq!(child_stage_status(&blocked, "split"), STATUS_BLOCKED);
    assert_eq!(child_stage_status(&blocked, "prepare"), STATUS_PENDING);
    assert_eq!(
        blocked.children[0].last_error.as_deref(),
        Some("source_changed_downstream_exists")
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn split_policy_upgrade_does_not_cancel_an_agent_owned_handoff() {
    let root = temp_root("split-policy-agent-handoff");
    let repo = handoff_repo_fixture(&root);
    let store = BookPipelineStore::for_test(&root);
    let job_id = fake_handoff_ready_job(&store, &repo);
    configure_expert_job(&store, &job_id);

    advance_job(&store, &job_id, None, false).unwrap();
    advance_job(&store, &job_id, None, false).unwrap();
    approve_ready_translation_for_test(&store, &job_id);

    let mut state = store.load().unwrap();
    let child = &mut state
        .jobs
        .iter_mut()
        .find(|job| job.id == job_id)
        .unwrap()
        .children[0];
    set_agent_handoff_waiting(child, "translate", "fake-agent-profile");
    child
        .stages
        .iter_mut()
        .find(|stage| stage.stage_id == "split")
        .unwrap()
        .input_hashes
        .insert("splitPolicyVersion".into(), "split-policy-obsolete".into());
    store.save(&state).unwrap();

    let state = store.load().unwrap();
    let child = &state
        .jobs
        .iter()
        .find(|job| job.id == job_id)
        .unwrap()
        .children[0];
    assert!(evaluate_split_freshness(child, false).unwrap().is_none());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn retry_button_routes_a_staged_failure_to_advance_without_reextracting() {
    let root = temp_root("retry-staged-failure");
    let repo = handoff_repo_fixture(&root);
    let source_path = root.join("source.md");
    let store = BookPipelineStore::for_test(&root);
    let job_id = handoff_ready_child_job(
        &store,
        &repo,
        &source_path,
        "# One\n\nBody one.\n\n# Two\n\nBody two.\n",
    );

    advance_job(&store, &job_id, None, false).unwrap();
    advance_job(&store, &job_id, None, false).unwrap();
    approve_ready_translation_for_test(&store, &job_id);

    let mut state = store.load().unwrap();
    let owner = store.execution_owner().unwrap().to_string();
    let child = &mut state
        .jobs
        .iter_mut()
        .find(|job| job.id == job_id)
        .unwrap()
        .children[0];
    start_stage(child, "translate", &owner);
    store.save(&state).unwrap();

    let mut state = store.load().unwrap();
    let child = &mut state
        .jobs
        .iter_mut()
        .find(|job| job.id == job_id)
        .unwrap()
        .children[0];
    set_stage_status(
        child,
        "translate",
        STATUS_FAILED,
        Some("simulated staged failure".into()),
    );
    child
        .stages
        .iter_mut()
        .find(|stage| stage.stage_id == "split")
        .unwrap()
        .input_hashes
        .insert("splitPolicyVersion".into(), "split-policy-obsolete".into());
    store.save(&state).unwrap();

    let retried = retry_job_from_ui(&store, &SystemPipelineRunner, &job_id).unwrap();

    assert_eq!(child_stage_status(&retried, "split"), STATUS_COMPLETED);
    assert_eq!(child_stage_status(&retried, "prepare"), STATUS_PENDING);
    assert_eq!(child_stage_status(&retried, "translate"), STATUS_PENDING);
    assert_eq!(stage_attempt(&retried, "extract"), 1);
    assert_eq!(
        retried.children[0]
            .stages
            .iter()
            .find(|stage| stage.stage_id == "split")
            .unwrap()
            .input_hashes
            .get("splitPolicyVersion")
            .map(String::as_str),
        Some(SPLIT_POLICY_VERSION)
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn split_policy_invalidation_marker_cannot_authorize_unrelated_stage_regressions() {
    let root = temp_root("split-policy-invalidation-scope");
    let store = BookPipelineStore::for_test(&root);
    let job = queue_job(
        &store,
        fake_source(None),
        "conversion_only".into(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();
    let base = job.children[0]
        .stages
        .iter()
        .find(|stage| stage.stage_id == "extract")
        .unwrap()
        .clone();

    let assert_rejected = |stage_id: &str, version: &str, extra_hash: bool| {
        let mut previous = base.clone();
        previous.stage_id = stage_id.into();
        previous.status = STATUS_FAILED.into();
        let mut next = previous.clone();
        next.status = STATUS_PENDING.into();
        next.input_hashes.clear();
        next.input_hashes
            .insert("splitPolicyVersion".into(), version.into());
        if extra_hash {
            next.input_hashes
                .insert("unexpected".into(), "value".into());
        }
        assert!(!is_allowed_stage_transition(&previous, &next));
    };

    assert_rejected("extract", SPLIT_POLICY_VERSION, false);
    assert_rejected("prepare", "split-policy-spoofed", false);
    assert_rejected("prepare", SPLIT_POLICY_VERSION, true);
    let _ = fs::remove_dir_all(root);
}

#[test]
fn advanced_split_and_prepare_survive_store_restart() {
    let root = temp_root("advance-restart");
    let repo = handoff_repo_fixture(&root);
    let source_path = root.join("source.md");
    let job_id = {
        let store = BookPipelineStore::for_test(&root);
        let job_id = handoff_ready_child_job(
            &store,
            &repo,
            &source_path,
            "# Alpha\n\nAlpha body.\n\n# Beta\n\nBeta body.\n",
        );
        advance_job(&store, &job_id, None, false).unwrap();
        advance_job(&store, &job_id, None, false).unwrap();
        job_id
    };

    let reopened = BookPipelineStore::for_test(&root);
    let state = reopened.load().unwrap();
    let job = state.jobs.iter().find(|job| job.id == job_id).unwrap();
    let child = &job.children[0];
    assert_eq!(
        child
            .stages
            .iter()
            .find(|stage| stage.stage_id == "split")
            .unwrap()
            .status,
        STATUS_COMPLETED
    );
    assert_eq!(
        child
            .stages
            .iter()
            .find(|stage| stage.stage_id == "prepare")
            .unwrap()
            .status,
        STATUS_COMPLETED
    );
    assert!(child
        .artifacts
        .iter()
        .any(|artifact| artifact.kind == "source_map" && artifact.sha256.is_some()));
    assert_eq!(
        child
            .artifacts
            .iter()
            .filter(|artifact| artifact.kind == "translation_task_manifest")
            .count(),
        2
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn split_and_prepare_keep_private_text_out_of_job_records() {
    let root = temp_root("advance-privacy");
    let repo = handoff_repo_fixture(&root);
    let source_path = root.join("source.md");
    let store = BookPipelineStore::for_test(&root);
    let job_id = handoff_ready_child_job(
        &store,
        &repo,
        &source_path,
        "# Distinctivechaptertitle\n\nConfidentialbodytext lives here.\n",
    );

    advance_job(&store, &job_id, None, false).unwrap();
    let advanced = advance_job(&store, &job_id, None, false).unwrap();

    let log = advanced.log_summary.join("\n");
    assert!(!log.contains("Distinctivechaptertitle"));
    assert!(!log.contains("Confidentialbodytext"));
    for stage in &advanced.children[0].stages {
        if let Some(error) = &stage.error {
            assert!(!error.contains("Confidentialbodytext"));
        }
    }
    for artifact in &advanced.children[0].artifacts {
        assert!(!artifact.path.contains("Confidentialbodytext"));
        assert!(!artifact.path.contains("Distinctivechaptertitle"));
    }

    // Traceability metadata still lives in the local (gitignored) source map.
    let source_map = fs::read_to_string(
        child_project_root(&advanced)
            .join("metadata")
            .join("source_map.json"),
    )
    .unwrap();
    assert!(source_map.contains("Distinctivechaptertitle"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn persisted_file_artifacts_have_complete_immutable_provenance() {
    let root = temp_root("artifact-provenance");
    let input = root.join("input");
    fs::create_dir_all(&input).unwrap();
    fs::write(input.join("sample.pdf"), "%PDF fixture").unwrap();
    let store = BookPipelineStore::for_test(&root);
    let job = queue_job(
        &store,
        local_pdf_source(&input),
        "conversion_only".into(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();

    let completed = run_job(&store, &ArtifactFixtureRunner, &job.id).unwrap();
    let markdown = completed
        .artifacts
        .iter()
        .find(|artifact| artifact.kind == "markdown")
        .expect("registered Markdown artifact");
    let markdown_path = PathBuf::from(&markdown.path);
    assert!(markdown.artifact_id.starts_with("artifact-"));
    assert_eq!(
        markdown.sha256.as_deref(),
        Some(sha256_file(&markdown_path).unwrap().as_str())
    );
    assert_eq!(
        markdown.size_bytes,
        Some(fs::metadata(&markdown_path).unwrap().len())
    );
    assert_eq!(markdown.producer.stage_id, "extract");
    assert_eq!(markdown.producer.attempt, 1);
    assert_eq!(
        markdown.producer.child_job_id.as_deref(),
        Some(completed.children[0].id.as_str())
    );
    assert!(!markdown.input_hashes.is_empty());
    assert!(!markdown.source_refs.source_ref_sha256.is_empty());
    assert_eq!(markdown.privacy, "private_text");
    assert!(markdown.validation.exists);
    assert!(markdown.validation.nonempty);
    assert!(markdown.validation.hash_matches);
    assert!(completed
        .artifacts
        .iter()
        .all(|artifact| Path::new(&artifact.path).is_file()));
    assert!(completed
        .children
        .iter()
        .flat_map(|child| child.stages.iter())
        .find(|stage| stage.stage_id == "extract")
        .is_some_and(|stage| stage.artifact_ids.contains(&markdown.artifact_id)));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn persisted_artifact_identity_rejects_producer_mutation() {
    let root = temp_root("artifact-immutable-producer");
    let store = BookPipelineStore::for_test(&root);
    let queued = queue_job(
        &store,
        fake_source(None),
        "conversion_only".into(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();
    let completed = run_job(&store, &ArtifactFixtureRunner, &queued.id).unwrap();
    let artifact_id = completed
        .artifacts
        .iter()
        .find(|artifact| artifact.kind == "markdown")
        .unwrap()
        .artifact_id
        .clone();
    let mut state = store.load().unwrap();
    let job = state
        .jobs
        .iter_mut()
        .find(|job| job.id == queued.id)
        .unwrap();
    for artifact in job
        .artifacts
        .iter_mut()
        .chain(
            job.children
                .iter_mut()
                .flat_map(|child| child.artifacts.iter_mut()),
        )
        .chain(
            job.collection_items
                .iter_mut()
                .flat_map(|item| item.artifacts.iter_mut()),
        )
        .filter(|artifact| artifact.artifact_id == artifact_id)
    {
        artifact.producer.attempt += 1;
    }

    let error = store.save(&state).unwrap_err();

    assert!(error.contains("was mutated"));
    assert!(error.contains("producer"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn worker_output_persists_only_allowlisted_markers() {
    let root = temp_root("worker-marker-allowlist");
    let output = root.join("job-output");
    fs::create_dir_all(&output).unwrap();
    let hash = "a".repeat(64);
    let text = format!(
        "private source paragraph\nBOOK_PIPELINE_MARKER status=completed count=2 sha256={hash} path={}\nprompt=translate this secret\nhttps://private.example/file?X-Amz-Signature=secret",
        display_path(&output.join("result.md"))
    );

    let markers = parse_allowlisted_worker_markers(&text, &[output.as_path()]);

    assert_eq!(markers.len(), 1);
    assert!(markers[0].contains("status=completed"));
    assert!(markers[0].contains("count=2"));
    assert!(markers[0].contains(&hash));
    let persisted = markers.join("\n");
    assert!(!persisted.contains("private source paragraph"));
    assert!(!persisted.contains("prompt="));
    assert!(!persisted.contains("Signature"));
    assert!(!persisted.contains("secret"));
    let _ = fs::remove_dir_all(root);
}

#[test]
fn structured_errors_classify_and_redact_sensitive_payloads() {
    let error = safe_error_from_message(
        "extract",
        None,
        3,
        "ZOTERO_API_KEY=supersecret Authorization: Bearer nope https://private.example/file?X-Amz-Signature=secret prompt=private source text",
    );

    assert_eq!(error.code, "missing_credentials");
    assert_eq!(error.stage_id, "extract");
    assert_eq!(error.attempt, 3);
    assert!(error.retryable);
    assert!(!error.summary.contains("supersecret"));
    assert!(!error.summary.contains("private.example"));
    assert!(!error.summary.contains("prompt"));
    assert!(!error.summary.contains("source text"));
}

#[test]
fn a_missing_key_message_names_no_secret_and_stays_legible() {
    let error = safe_error_from_message(
        "index",
        None,
        1,
        "Zotero item-scoped full-text index exited with status 1: RuntimeError: GOOGLE_API_KEY or GEMINI_API_KEY not set.",
    );

    assert_eq!(error.code, "missing_credentials");
    assert_eq!(
        error.summary,
        "Zotero item-scoped full-text index exited with status 1: RuntimeError: GOOGLE_API_KEY or GEMINI_API_KEY not set."
    );
}

#[test]
fn diagnostic_profiles_have_monotonic_disclosure() {
    let root = temp_root("diagnostic-profiles");
    let store = BookPipelineStore::for_test(&root);
    let job = queue_job(
        &store,
        fake_source(None),
        "conversion_only".into(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();
    let completed = run_job(&store, &ArtifactFixtureRunner, &job.id).unwrap();

    let local =
        serde_json::to_string(&build_book_pipeline_diagnostic(&completed, "local-full").unwrap())
            .unwrap();
    let support = serde_json::to_string(
        &build_book_pipeline_diagnostic(&completed, "redacted-support").unwrap(),
    )
    .unwrap();
    let public =
        serde_json::to_string(&build_book_pipeline_diagnostic(&completed, "public-issue").unwrap())
            .unwrap();

    assert!(local.contains(&completed.artifacts[0].path));
    assert!(local.contains(completed.artifacts[0].sha256.as_deref().unwrap()));
    assert!(!support.contains(&display_path(&root)));
    assert!(support.contains("<JOB_ROOT>"));
    assert!(support.contains(completed.artifacts[0].sha256.as_deref().unwrap()));
    assert!(!public.contains(&display_path(&root)));
    assert!(!public.contains(completed.artifacts[0].sha256.as_deref().unwrap()));
    assert!(!public.contains("artifactId"));
    for export in [&local, &support, &public] {
        assert!(!export.contains("stdout"));
        assert!(!export.contains("stderr"));
        assert!(!export.contains("providerPayload"));
        assert!(!export.contains("prompt"));
    }
    let _ = fs::remove_dir_all(root);
}

#[test]
fn diagnostic_bundle_lands_in_the_chosen_folder_under_a_contained_name() {
    // The disclosure test above covers what goes in; this covers the part
    // that makes it reachable at all -- the bundle has to be a file the user
    // can attach to a report, not a value returned in-process.
    let root = temp_root("diagnostic-write");
    let store = BookPipelineStore::for_test(&root);
    let job = queue_job(
        &store,
        fake_source(None),
        "conversion_only".into(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();
    let completed = run_job(&store, &ArtifactFixtureRunner, &job.id).unwrap();
    let out = root.join("export");
    fs::create_dir_all(&out).unwrap();

    let document = build_book_pipeline_diagnostic(&completed, "public-issue").unwrap();
    let path =
        write_book_pipeline_diagnostic(&out, &completed.id, "public-issue", &document).unwrap();
    assert_eq!(path.parent(), Some(out.as_path()));
    assert_eq!(
        path.file_name().and_then(|name| name.to_str()),
        Some(format!("bibliosmith-diagnostic-{}-public-issue.json", completed.id).as_str())
    );
    let written: serde_json::Value =
        serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(written["profile"], "public-issue");
    assert_eq!(written, document);

    // A job id that is not filename-safe must not steer the write out of the
    // folder the user picked.
    let escaped =
        write_book_pipeline_diagnostic(&out, "../../etc/pwned", "public-issue", &document).unwrap();
    assert_eq!(escaped.parent(), Some(out.as_path()));
    assert_eq!(
        escaped.file_name().and_then(|name| name.to_str()),
        Some("bibliosmith-diagnostic-______etc_pwned-public-issue.json")
    );

    assert!(build_book_pipeline_diagnostic(&completed, "everything").is_err());
    let _ = fs::remove_dir_all(root);
}

fn open_target_fixture(
    root: &Path,
    status: &str,
    kind: &str,
    action_label: &str,
) -> BookPipelineJob {
    let store = BookPipelineStore::for_test(root);
    let mut job = queue_job(
        &store,
        fake_source(None),
        "conversion_only".into(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();
    let target_root = root.join("registered-job-root");
    fs::create_dir_all(&target_root).unwrap();
    let target_path = if kind.ends_with("_directory") || kind == "workspace" {
        target_root.join(kind)
    } else {
        target_root.join(format!("{kind}.json"))
    };
    if target_path.extension().is_some() {
        fs::write(&target_path, "fixture").unwrap();
    } else {
        fs::create_dir_all(&target_path).unwrap();
    }
    job.status = status.into();
    job.navigation_targets = vec![BookPipelineNavigationTarget {
        target_id: format!("target-{kind}"),
        kind: kind.into(),
        path: display_path(&target_path),
        allowed_root: display_path(&target_root),
        artifact_id: None,
    }];
    job.open_target = select_book_pipeline_open_target(&job);
    assert_eq!(
        job.open_target
            .as_ref()
            .map(|target| target.action_label.as_str()),
        Some(action_label)
    );
    job
}

#[test]
fn every_job_status_resolves_a_deterministic_registered_open_target() {
    let cases = [
        (STATUS_PENDING, "workspace", "Open workspace"),
        (STATUS_READY, "workspace", "Open workspace"),
        (STATUS_RUNNING, "workspace", "Open workspace"),
        (
            STATUS_WAITING_FOR_APPROVAL,
            "approval_packet",
            "Review approval",
        ),
        (STATUS_BLOCKED, "blocker_evidence", "Review blocker"),
        (STATUS_FAILED, "failure_evidence", "Open failure evidence"),
        (STATUS_PARTIAL, "partial_results", "Inspect partial results"),
        (
            STATUS_COMPLETED,
            "reading_output_directory",
            "Open reading output",
        ),
        (
            STATUS_SKIPPED,
            "verified_evidence",
            "Open verified evidence",
        ),
    ];
    for (index, (status, kind, action_label)) in cases.into_iter().enumerate() {
        let root = temp_root(&format!("open-status-{index}"));
        let job = open_target_fixture(&root, status, kind, action_label);
        let allowed = root.join("registered-job-root");

        let resolved = resolve_book_pipeline_open_target(&job, &[allowed]).unwrap();

        assert_eq!(resolved.kind, kind);
        assert_eq!(resolved.action_label, action_label);
        assert!(resolved.path.exists());
        let _ = fs::remove_dir_all(root);
    }
}

#[test]
fn completed_collection_uses_collection_results_action() {
    let root = temp_root("open-collection-results");
    let mut job = open_target_fixture(&root, STATUS_PENDING, "workspace", "Open workspace");
    job.kind = "collection".into();
    job.status = STATUS_COMPLETED.into();
    job.navigation_targets[0].kind = "collection_results".into();
    job.open_target = select_book_pipeline_open_target(&job);

    assert_eq!(
        job.open_target
            .as_ref()
            .map(|target| target.action_label.as_str()),
        Some("Open collection results")
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn skipped_collection_opens_its_hashed_manifest_as_verified_evidence() {
    let root = temp_root("open-skipped-collection");
    let store = BookPipelineStore::for_test(&root);
    let mut job = queue_job(
        &store,
        fake_collection_source(),
        "conversion_only".into(),
        BookPipelinePreviewConfig {
            has_paddleocr_credentials: true,
            has_mineru_credentials: true,
            route_overrides: BTreeMap::new(),
        },
    )
    .unwrap();
    for stage in &mut job.stages {
        stage.status = STATUS_COMPLETED.into();
    }
    for child in &mut job.children {
        for stage in &mut child.stages {
            stage.status = STATUS_SKIPPED.into();
        }
    }
    let output_dir = PathBuf::from(job.output_dir.as_deref().unwrap());
    let manifest_path = output_dir.join("collection-summary.json");
    fs::write(&manifest_path, "{\"schema\":\"fixture\"}\n").unwrap();
    job.artifacts
        .push(required_stage_artifact("collection_manifest", &manifest_path, "discover").unwrap());

    derive_job(&mut job);
    let resolved = resolve_book_pipeline_open_target(&job, &[output_dir]).unwrap();

    assert_eq!(job.status, STATUS_SKIPPED);
    assert_eq!(resolved.kind, "verified_evidence");
    assert_eq!(resolved.action_label, "Open verified evidence");
    assert_eq!(resolved.path, fs::canonicalize(manifest_path).unwrap());
    let _ = fs::remove_dir_all(root);
}

#[test]
fn open_target_rejects_traversal_missing_paths_and_source_pdf_fallback() {
    let root = temp_root("open-target-guardrails");
    let allowed = root.join("registered-job-root");
    fs::create_dir_all(&allowed).unwrap();
    let escaped = root.join("escaped.txt");
    fs::write(&escaped, "outside").unwrap();
    let mut job = open_target_fixture(
        &root,
        STATUS_FAILED,
        "failure_evidence",
        "Open failure evidence",
    );
    job.navigation_targets[0].path = display_path(&allowed.join("..").join("escaped.txt"));
    assert!(
        resolve_book_pipeline_open_target(&job, std::slice::from_ref(&allowed))
            .unwrap_err()
            .contains("open_target_invalid")
    );

    job.navigation_targets[0].path = display_path(&allowed.join("missing.json"));
    assert!(
        resolve_book_pipeline_open_target(&job, std::slice::from_ref(&allowed))
            .unwrap_err()
            .contains("open_target_invalid")
    );

    let source_pdf = root.join("private-source.pdf");
    fs::write(&source_pdf, "%PDF private").unwrap();
    job.source.path = Some(display_path(&source_pdf));
    job.navigation_targets.clear();
    job.open_target = None;
    assert!(
        resolve_book_pipeline_open_target(&job, std::slice::from_ref(&allowed))
            .unwrap_err()
            .contains("open_target_invalid")
    );
    let _ = fs::remove_dir_all(root);
}

fn excerpt_fixture_job(artifact_path: &Path) -> BookPipelineJob {
    serde_json::from_value(serde_json::json!({
        "id": "job-excerpt",
        "mode": "convert_then_translate",
        "source": { "kind": "local_pdf_folder", "title": "Fixture" },
        "route": [],
        "status": "waiting_for_approval",
        "currentStep": "approve_translation",
        "lastError": null,
        "logSummary": [],
        "artifacts": [{
            "artifactId": "art-1",
            "kind": "extraction_markdown",
            "path": display_path(artifact_path),
            "sha256": null,
            "zoteroKey": null
        }],
        "outputDir": null,
        "attempts": 1,
        "createdAt": "2026-07-18T00:00:00Z",
        "updatedAt": "2026-07-18T00:00:00Z"
    }))
    .unwrap()
}

#[test]
fn artifact_excerpt_returns_truncated_head_within_allowlist() {
    let root = std::env::temp_dir().join(format!("bp-excerpt-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    let artifact_path = root.join("book.md");
    let body = format!("# Title\n\n{}", "正文abc".repeat(400));
    fs::write(&artifact_path, &body).unwrap();
    let job = excerpt_fixture_job(&artifact_path);

    let excerpt =
        read_artifact_excerpt(&job, "art-1", Some(64), std::slice::from_ref(&root)).unwrap();
    assert_eq!(excerpt.artifact_id, "art-1");
    assert_eq!(excerpt.kind, "extraction_markdown");
    assert!(excerpt.truncated);
    assert_eq!(excerpt.excerpt.chars().count(), 64);
    assert!(excerpt.excerpt.starts_with("# Title"));

    let full = read_artifact_excerpt(&job, "art-1", Some(4000), std::slice::from_ref(&root));
    assert!(full.is_ok());

    let _ = fs::remove_dir_all(root);
}

#[test]
fn artifact_excerpt_rejects_paths_outside_allowlist_and_unknown_artifacts() {
    let root = std::env::temp_dir().join(format!("bp-excerpt-out-{}", std::process::id()));
    let elsewhere = std::env::temp_dir().join(format!("bp-excerpt-else-{}", std::process::id()));
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&elsewhere).unwrap();
    let artifact_path = elsewhere.join("secret.md");
    fs::write(&artifact_path, "secret body").unwrap();
    let job = excerpt_fixture_job(&artifact_path);

    let outside = read_artifact_excerpt(&job, "art-1", None, std::slice::from_ref(&root));
    assert!(outside.unwrap_err().contains("artifact_excerpt_invalid"));

    let unknown = read_artifact_excerpt(&job, "missing", None, std::slice::from_ref(&root));
    assert!(unknown.unwrap_err().contains("not registered"));

    let _ = fs::remove_dir_all(root);
    let _ = fs::remove_dir_all(elsewhere);
}

// ---- OCR dual-engine sampling (issue #97) ---------------------------------

/// Stands in for `packages/ocr/sample_compare.py`. It writes the report to the
/// path the manifest names rather than returning it on stdout, because that
/// on-disk hand-off is the contract under test: unlike the conversion workers,
/// this report has to outlive the process that produced it.
struct OcrSampleFixtureExecutor {
    manifests: Mutex<Vec<serde_json::Value>>,
    paddle_markdown: String,
    mineru_markdown: String,
    paddle_error: Option<String>,
    total_pages: u32,
}

impl Default for OcrSampleFixtureExecutor {
    fn default() -> Self {
        Self {
            manifests: Mutex::new(Vec::new()),
            paddle_markdown: "# Paddle heading\n\nPaddle body.".into(),
            mineru_markdown: "# MinerU heading\n\nMinerU body.".into(),
            paddle_error: None,
            total_pages: 40,
        }
    }
}

impl OcrSampleFixtureExecutor {
    fn manifests(&self) -> Vec<serde_json::Value> {
        self.manifests.lock().unwrap().clone()
    }

    fn engine_entry(&self, engine: &str, markdown: &str, budget: usize) -> serde_json::Value {
        if engine == OCR_SAMPLE_ENGINE_PADDLEOCR {
            if let Some(error) = &self.paddle_error {
                return serde_json::json!({
                    "engine": engine,
                    "status": "failed",
                    "markdownExcerpt": "",
                    "characterCount": 0,
                    "elapsedMs": 12,
                    "error": error,
                });
            }
        }
        serde_json::json!({
            "engine": engine,
            "status": "ok",
            "markdownExcerpt": markdown.chars().take(budget).collect::<String>(),
            "characterCount": markdown.chars().count(),
            "elapsedMs": 34,
            "error": serde_json::Value::Null,
        })
    }
}

impl RunnerCommandExecutor for OcrSampleFixtureExecutor {
    fn execute(&self, command: &RunnerCommand) -> Result<RunnerCommandResult, String> {
        assert_eq!(command.kind, RunnerCommandKind::Process);
        assert_eq!(command.label, OCR_SAMPLE_COMPARE_COMMAND_LABEL);
        assert_eq!(command.program, PathBuf::from("uv"));
        assert_eq!(command.accepted_exit_codes, vec![0]);
        assert_eq!(command.args[0..4], ["run", "--package", "ocr", "python"]);
        assert!(
            command.args[4].ends_with("sample_compare.py"),
            "unexpected script: {}",
            command.args[4]
        );
        assert_eq!(command.args[5], "--manifest");
        // `uv run --package ocr` resolves the workspace from the OCR root, and
        // the script imports its sibling engine clients by bare name. Without
        // this the subprocess fails only against a real install.
        assert_eq!(
            command.cwd.as_deref(),
            Some(book_ocr_conversion_root().as_path())
        );
        // Whatever the Keychain holds is what the engines authenticate with;
        // dropping inject_ocr_credentials would otherwise fail nothing here and
        // report "not configured" for both engines on a configured machine.
        let injected = command
            .env
            .iter()
            .map(|(key, _)| key.as_str())
            .collect::<BTreeSet<_>>();
        for (key, _) in crate::ocr_settings::resolve_credential_env() {
            assert!(
                injected.contains(key.as_str()),
                "credential {key} was not injected"
            );
        }

        let manifest_path = PathBuf::from(&command.args[6]);
        let manifest: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(&manifest_path).unwrap()).unwrap();
        assert_eq!(manifest["schema"], OCR_SAMPLE_COMPARE_SCHEMA);
        assert_eq!(
            manifest["characterBudget"],
            serde_json::json!(OCR_SAMPLE_CHARACTER_BUDGET)
        );
        assert_eq!(
            manifest["engines"],
            serde_json::json!([OCR_SAMPLE_ENGINE_PADDLEOCR, OCR_SAMPLE_ENGINE_MINERU])
        );
        // Every key run_sample_manifest in packages/ocr/sample_compare.py
        // requires. Renaming or dropping one here fails nothing on this side
        // until a real book is sampled, where Python rejects the manifest.
        assert_eq!(
            manifest
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([
                "characterBudget",
                "engines",
                "projectRoot",
                "reportPath",
                "samplePages",
                "schema",
                "sourcePdfPath",
                "workDir",
            ])
        );
        // Python rejects a bool for these, and serde_json would happily make
        // one out of a mistyped Rust field.
        assert!(manifest["samplePages"].is_u64());
        assert!(manifest["characterBudget"].is_u64());
        // Relative to projectRoot; an absolute path here would let the report
        // escape the child's own sample directory.
        assert!(!Path::new(manifest["reportPath"].as_str().unwrap()).is_absolute());
        assert!(!Path::new(manifest["workDir"].as_str().unwrap()).is_absolute());
        self.manifests.lock().unwrap().push(manifest.clone());

        let project_root = PathBuf::from(manifest["projectRoot"].as_str().unwrap());
        let sample_pages = manifest["samplePages"].as_u64().unwrap() as u32;
        let budget = manifest["characterBudget"].as_u64().unwrap() as usize;
        let sampled: Vec<u32> = (1..=sample_pages)
            .map(|index| index * (self.total_pages / (sample_pages + 1)))
            .collect();
        let report = serde_json::json!({
            "schema": OCR_SAMPLE_COMPARE_REPORT_SCHEMA,
            "totalPages": self.total_pages,
            "sampledPages": sampled,
            "characterBudget": budget,
            "engines": [
                self.engine_entry(OCR_SAMPLE_ENGINE_PADDLEOCR, &self.paddle_markdown, budget),
                self.engine_entry(OCR_SAMPLE_ENGINE_MINERU, &self.mineru_markdown, budget),
            ],
        });
        let report_path = project_root.join(manifest["reportPath"].as_str().unwrap());
        fs::create_dir_all(report_path.parent().unwrap()).unwrap();
        fs::write(
            &report_path,
            serde_json::to_string_pretty(&report).unwrap() + "\n",
        )
        .unwrap();
        Ok(RunnerCommandResult {
            stdout: String::new(),
            stderr: String::new(),
            log_summary: vec!["OCR sample fixture completed".into()],
        })
    }
}

fn ocr_sample_ready_job(root: &Path, store: &MemoryStateStore) -> (String, String, PathBuf) {
    fs::create_dir_all(root).unwrap();
    let source_pdf = root.join("scanned.pdf");
    fs::write(&source_pdf, "%PDF fixture").unwrap();
    let job = queue_job(
        store,
        fake_direct_zotero_source(),
        MODE_CONVERT_THEN_TRANSLATE.into(),
        BookPipelinePreviewConfig::default(),
    )
    .unwrap();
    let child_id = job.children[0].id.clone();
    let mut state = store.load().unwrap();
    let stored = state.jobs.iter_mut().find(|it| it.id == job.id).unwrap();
    stored.children[0].source.path = Some(display_path(&source_pdf));
    store.save(&state).unwrap();
    (job.id, child_id, source_pdf)
}

#[test]
fn ocr_sample_registers_a_report_both_engines_answered() {
    let root = temp_root("ocr-sample-compare");
    let store = MemoryStateStore::new(&root);
    let (job_id, child_id, _) = ocr_sample_ready_job(&root, &store);
    let executor = OcrSampleFixtureExecutor::default();

    let job = run_ocr_sample_with_executor(
        &store,
        &job_id,
        Some(&child_id),
        OCR_SAMPLE_PAGE_COUNT,
        &executor,
    )
    .unwrap();

    let manifests = executor.manifests();
    assert_eq!(manifests.len(), 1);
    assert_eq!(manifests[0]["samplePages"], OCR_SAMPLE_PAGE_COUNT);
    assert!(manifests[0]["sourcePdfPath"]
        .as_str()
        .unwrap()
        .ends_with("scanned.pdf"));

    let artifact = job.children[0]
        .artifacts
        .iter()
        .find(|artifact| artifact.kind == "ocr_sample_report")
        .expect("the sample report should be registered");
    assert!(Path::new(&artifact.path).is_file());
    assert_eq!(
        artifact.sha256.as_deref(),
        Some(sha256_file(Path::new(&artifact.path)).unwrap().as_str())
    );

    let report =
        read_ocr_sample_report(&job, &child_id, &ocr_sample_dir(&store, &job_id, &child_id))
            .unwrap();
    assert_eq!(report.schema, OCR_SAMPLE_COMPARE_REPORT_SCHEMA);
    assert_eq!(report.sampled_pages.len(), OCR_SAMPLE_PAGE_COUNT as usize);
    assert_eq!(
        report
            .engines
            .iter()
            .map(|engine| engine.engine.as_str())
            .collect::<Vec<_>>(),
        vec![OCR_SAMPLE_ENGINE_PADDLEOCR, OCR_SAMPLE_ENGINE_MINERU]
    );
    assert!(report.engines.iter().all(|engine| engine.status == "ok"));
    assert!(report.engines[0].markdown_excerpt.contains("Paddle"));
    assert!(report.engines[1].markdown_excerpt.contains("MinerU"));

    let _ = fs::remove_dir_all(root);
}

// A sample the user cannot compare is worse than none: it costs the same API
// calls and leaves the route decision exactly where it was.
#[test]
fn ocr_sample_never_reports_a_cover_or_endpoint_page() {
    let root = temp_root("ocr-sample-endpoints");
    let store = MemoryStateStore::new(&root);
    let (job_id, child_id, _) = ocr_sample_ready_job(&root, &store);

    let job = run_ocr_sample_with_executor(
        &store,
        &job_id,
        Some(&child_id),
        OCR_SAMPLE_PAGE_COUNT,
        &OcrSampleFixtureExecutor::default(),
    )
    .unwrap();
    let report =
        read_ocr_sample_report(&job, &child_id, &ocr_sample_dir(&store, &job_id, &child_id))
            .unwrap();
    for page in &report.sampled_pages {
        assert!(
            *page >= 2 && *page < report.total_pages,
            "sampled an endpoint page: {page} of {}",
            report.total_pages
        );
    }

    // And the validator refuses one that did.
    let mut cover = report.clone();
    cover.sampled_pages = vec![1, 20, 30];
    assert!(validate_ocr_sample_report(&cover, OCR_SAMPLE_PAGE_COUNT)
        .unwrap_err()
        .contains("cover"));

    let _ = fs::remove_dir_all(root);
}

// One engine down should not throw away the other's answer -- the sampled
// pages were already paid for, and half a comparison still decides the route.
#[test]
fn ocr_sample_keeps_the_surviving_engine_when_the_other_fails() {
    let root = temp_root("ocr-sample-one-engine");
    let store = MemoryStateStore::new(&root);
    let (job_id, child_id, _) = ocr_sample_ready_job(&root, &store);
    let executor = OcrSampleFixtureExecutor {
        paddle_error: Some("BAIDU_PADDLEOCR_TOKEN is not configured".into()),
        ..OcrSampleFixtureExecutor::default()
    };

    let job = run_ocr_sample_with_executor(
        &store,
        &job_id,
        Some(&child_id),
        OCR_SAMPLE_PAGE_COUNT,
        &executor,
    )
    .unwrap();
    let report =
        read_ocr_sample_report(&job, &child_id, &ocr_sample_dir(&store, &job_id, &child_id))
            .unwrap();
    assert_eq!(report.engines[0].status, "failed");
    assert!(report.engines[0]
        .error
        .as_deref()
        .unwrap()
        .contains("BAIDU_PADDLEOCR_TOKEN"));
    assert_eq!(report.engines[1].status, "ok");

    // Both failing is a real failure, not an empty panel.
    let mut both_failed = report.clone();
    both_failed.engines[1].status = "failed".into();
    both_failed.engines[1].error = Some("MinerU quota exhausted".into());
    let err = validate_ocr_sample_report(&both_failed, OCR_SAMPLE_PAGE_COUNT).unwrap_err();
    assert!(err.contains("Both OCR engines failed"), "{err}");
    assert!(err.contains("MinerU quota exhausted"), "{err}");

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ocr_sample_replaces_the_previous_report_and_re_reads_it() {
    let root = temp_root("ocr-sample-resample");
    let store = MemoryStateStore::new(&root);
    let (job_id, child_id, _) = ocr_sample_ready_job(&root, &store);

    let first = run_ocr_sample_with_executor(
        &store,
        &job_id,
        Some(&child_id),
        OCR_SAMPLE_PAGE_COUNT,
        &OcrSampleFixtureExecutor::default(),
    )
    .unwrap();
    let first_report = read_ocr_sample_report(
        &first,
        &child_id,
        &ocr_sample_dir(&store, &job_id, &child_id),
    )
    .unwrap();

    let executor = OcrSampleFixtureExecutor {
        mineru_markdown: "# Second pass\n\nRe-sampled body.".into(),
        ..OcrSampleFixtureExecutor::default()
    };
    let second =
        run_ocr_sample_with_executor(&store, &job_id, Some(&child_id), 5, &executor).unwrap();

    assert_eq!(executor.manifests()[0]["samplePages"], 5);
    assert_eq!(
        second.children[0]
            .artifacts
            .iter()
            .filter(|artifact| artifact.kind == "ocr_sample_report")
            .count(),
        1,
        "re-sampling should replace the report, not accumulate reports"
    );
    let second_report = read_ocr_sample_report(
        &second,
        &child_id,
        &ocr_sample_dir(&store, &job_id, &child_id),
    )
    .unwrap();
    assert_ne!(first_report, second_report);
    assert_eq!(second_report.sampled_pages.len(), 5);
    assert!(second_report.engines[1]
        .markdown_excerpt
        .contains("Second pass"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ocr_sample_refuses_once_conversion_is_under_way() {
    let root = temp_root("ocr-sample-too-late");
    let store = MemoryStateStore::new(&root);
    let (job_id, child_id, _) = ocr_sample_ready_job(&root, &store);

    for status in [STATUS_RUNNING, STATUS_COMPLETED] {
        let mut state = store.load().unwrap();
        let stored = state.jobs.iter_mut().find(|it| it.id == job_id).unwrap();
        stage_mut(&mut stored.children[0], "extract")
            .unwrap()
            .status = status.into();
        store.save(&state).unwrap();

        let err = run_ocr_sample_with_executor(
            &store,
            &job_id,
            Some(&child_id),
            OCR_SAMPLE_PAGE_COUNT,
            &OcrSampleFixtureExecutor::default(),
        )
        .unwrap_err();
        assert!(err.contains("before conversion starts"), "{status}: {err}");
    }

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ocr_sample_rejects_unusable_inputs() {
    let root = temp_root("ocr-sample-inputs");
    let store = MemoryStateStore::new(&root);
    let (job_id, child_id, source_pdf) = ocr_sample_ready_job(&root, &store);
    let executor = OcrSampleFixtureExecutor::default();

    for pages in [0, OCR_SAMPLE_MAX_PAGES + 1] {
        let err = run_ocr_sample_with_executor(&store, &job_id, Some(&child_id), pages, &executor)
            .unwrap_err();
        assert!(err.contains("between 1 and"), "{pages}: {err}");
    }

    fs::remove_file(&source_pdf).unwrap();
    let err = run_ocr_sample_with_executor(
        &store,
        &job_id,
        Some(&child_id),
        OCR_SAMPLE_PAGE_COUNT,
        &executor,
    )
    .unwrap_err();
    assert!(err.contains("source PDF"), "{err}");

    let mut state = store.load().unwrap();
    let stored = state.jobs.iter_mut().find(|it| it.id == job_id).unwrap();
    stored.children[0].source.path = None;
    stored.children[0].route.clear();
    store.save(&state).unwrap();
    let err = run_ocr_sample_with_executor(
        &store,
        &job_id,
        Some(&child_id),
        OCR_SAMPLE_PAGE_COUNT,
        &executor,
    )
    .unwrap_err();
    assert!(err.contains("no local PDF to sample"), "{err}");

    assert!(
        executor.manifests().is_empty(),
        "a rejected sample must not spend an API call"
    );
    let _ = fs::remove_dir_all(root);
}

#[test]
fn ocr_sample_report_is_refused_when_it_changed_after_registration() {
    let root = temp_root("ocr-sample-tamper");
    let store = MemoryStateStore::new(&root);
    let (job_id, child_id, _) = ocr_sample_ready_job(&root, &store);
    let job = run_ocr_sample_with_executor(
        &store,
        &job_id,
        Some(&child_id),
        OCR_SAMPLE_PAGE_COUNT,
        &OcrSampleFixtureExecutor::default(),
    )
    .unwrap();
    let path = job.children[0]
        .artifacts
        .iter()
        .find(|artifact| artifact.kind == "ocr_sample_report")
        .unwrap()
        .path
        .clone();

    fs::write(&path, "{\"schema\":\"ocr-sample-compare-report-v1\"}\n").unwrap();
    let err = read_ocr_sample_report(&job, &child_id, &ocr_sample_dir(&store, &job_id, &child_id))
        .unwrap_err();
    assert!(err.contains("changed after registration"), "{err}");

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ocr_sample_report_excerpts_stay_out_of_the_log() {
    let root = temp_root("ocr-sample-privacy");
    let store = MemoryStateStore::new(&root);
    let (job_id, child_id, _) = ocr_sample_ready_job(&root, &store);
    let executor = OcrSampleFixtureExecutor {
        paddle_markdown: "Confidential page body from the scan.".into(),
        ..OcrSampleFixtureExecutor::default()
    };

    let job = run_ocr_sample_with_executor(
        &store,
        &job_id,
        Some(&child_id),
        OCR_SAMPLE_PAGE_COUNT,
        &executor,
    )
    .unwrap();

    assert_eq!(artifact_privacy("ocr_sample_report"), "private_text");
    assert_eq!(artifact_default_stage("ocr_sample_report"), "extract");
    for line in &job.log_summary {
        assert!(
            !line.contains("Confidential page body"),
            "the sampled text leaked into the log: {line}"
        );
    }

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ocr_sample_report_rejects_a_drifted_shape() {
    let base = BookPipelineOcrSampleReport {
        schema: OCR_SAMPLE_COMPARE_REPORT_SCHEMA.into(),
        total_pages: 40,
        sampled_pages: vec![10, 20, 30],
        character_budget: OCR_SAMPLE_CHARACTER_BUDGET,
        engines: vec![
            BookPipelineOcrSampleEngine {
                engine: OCR_SAMPLE_ENGINE_PADDLEOCR.into(),
                status: "ok".into(),
                markdown_excerpt: "paddle".into(),
                character_count: 6,
                elapsed_ms: 1,
                error: None,
            },
            BookPipelineOcrSampleEngine {
                engine: OCR_SAMPLE_ENGINE_MINERU.into(),
                status: "ok".into(),
                markdown_excerpt: "mineru".into(),
                character_count: 6,
                elapsed_ms: 1,
                error: None,
            },
        ],
    };
    assert!(validate_ocr_sample_report(&base, OCR_SAMPLE_PAGE_COUNT).is_ok());

    let mut wrong_schema = base.clone();
    wrong_schema.schema = "ocr-sample-compare-report-v0".into();
    assert!(validate_ocr_sample_report(&wrong_schema, OCR_SAMPLE_PAGE_COUNT).is_err());

    let mut unordered = base.clone();
    unordered.sampled_pages = vec![20, 10, 30];
    assert!(validate_ocr_sample_report(&unordered, OCR_SAMPLE_PAGE_COUNT).is_err());

    let mut too_many = base.clone();
    too_many.sampled_pages = vec![10, 15, 20, 25];
    assert!(validate_ocr_sample_report(&too_many, OCR_SAMPLE_PAGE_COUNT).is_err());

    let mut one_engine = base.clone();
    one_engine.engines.truncate(1);
    assert!(
        validate_ocr_sample_report(&one_engine, OCR_SAMPLE_PAGE_COUNT)
            .unwrap_err()
            .contains("both engines")
    );

    let mut swapped = base.clone();
    swapped.engines.reverse();
    assert!(validate_ocr_sample_report(&swapped, OCR_SAMPLE_PAGE_COUNT).is_err());

    let mut silent_failure = base.clone();
    silent_failure.engines[0].status = "failed".into();
    assert!(
        validate_ocr_sample_report(&silent_failure, OCR_SAMPLE_PAGE_COUNT)
            .unwrap_err()
            .contains("no reason")
    );

    let mut over_budget = base.clone();
    over_budget.character_budget = 3;
    assert!(
        validate_ocr_sample_report(&over_budget, OCR_SAMPLE_PAGE_COUNT)
            .unwrap_err()
            .contains("excerpt budget")
    );

    // deny_unknown_fields keeps a future engine field from being read as valid.
    let drifted = serde_json::to_value(&base)
        .map(|mut value| {
            value["engines"][0]["confidence"] = serde_json::json!(0.9);
            value
        })
        .unwrap();
    assert!(serde_json::from_value::<BookPipelineOcrSampleReport>(drifted).is_err());
}

/// Captured verbatim from a real run of `packages/ocr/sample_compare.py` with
/// stubbed engines. The fixture executor above writes a report of its own
/// shape, so without a sample of the real writer nothing here would notice the
/// two drifting apart.
///
/// What this catches is Rust-side drift against a known-good payload: a renamed
/// or newly required field, or a narrowed type. It cannot catch Python-side
/// drift -- it is a frozen literal, and nothing regenerates it. That direction
/// is covered by `ReportContractTests` in
/// packages/ocr/tests/test_sample_compare.py, which asserts the field set and
/// the value types of both the success and the failure branch against this same
/// struct.
// r##"..."## rather than r#"..."#: the captured Markdown starts with a heading,
// so the payload contains the sequence that would close the shorter delimiter.
const REAL_OCR_SAMPLE_REPORT: &str = r##"{
  "schema": "ocr-sample-compare-report-v1",
  "totalPages": 40,
  "sampledPages": [11, 21, 30],
  "characterBudget": 4000,
  "engines": [
    {
      "engine": "paddleocr",
      "status": "ok",
      "markdownExcerpt": "# Paddle heading\n\nPaddle body.",
      "characterCount": 30,
      "elapsedMs": 0,
      "error": null
    },
    {
      "engine": "mineru",
      "status": "failed",
      "markdownExcerpt": "",
      "characterCount": 0,
      "elapsedMs": 0,
      "error": "RuntimeError: MINERU_API_TOKEN is not configured"
    }
  ]
}"##;

#[test]
fn a_real_python_sample_report_deserializes_and_validates() {
    let report: BookPipelineOcrSampleReport =
        serde_json::from_str(REAL_OCR_SAMPLE_REPORT).expect("the Python writer's own output");
    assert_eq!(report.schema, OCR_SAMPLE_COMPARE_REPORT_SCHEMA);
    assert_eq!(report.sampled_pages, vec![11, 21, 30]);
    assert_eq!(report.engines[0].engine, OCR_SAMPLE_ENGINE_PADDLEOCR);
    assert_eq!(report.engines[1].engine, OCR_SAMPLE_ENGINE_MINERU);
    validate_ocr_sample_report(&report, OCR_SAMPLE_PAGE_COUNT).unwrap();
}

/// A failing re-sample used to overwrite the report the previous run had
/// registered, so one bad retry destroyed the last working comparison and left
/// the artifact record pointing at content whose digest no longer matched.
#[test]
fn a_failed_resample_leaves_the_previous_comparison_readable() {
    let root = temp_root("ocr-sample-failed-retry");
    let store = MemoryStateStore::new(&root);
    let (job_id, child_id, _) = ocr_sample_ready_job(&root, &store);
    let sample_dir = ocr_sample_dir(&store, &job_id, &child_id);

    let good = run_ocr_sample_with_executor(
        &store,
        &job_id,
        Some(&child_id),
        OCR_SAMPLE_PAGE_COUNT,
        &OcrSampleFixtureExecutor::default(),
    )
    .unwrap();
    let good_report = read_ocr_sample_report(&good, &child_id, &sample_dir).unwrap();

    struct FailingExecutor;
    impl RunnerCommandExecutor for FailingExecutor {
        fn execute(&self, _command: &RunnerCommand) -> Result<RunnerCommandResult, String> {
            Err("MinerU quota exhausted".into())
        }
    }
    let err = run_ocr_sample_with_executor(
        &store,
        &job_id,
        Some(&child_id),
        OCR_SAMPLE_PAGE_COUNT,
        &FailingExecutor,
    )
    .unwrap_err();
    assert!(err.contains("quota exhausted"), "{err}");

    let after = store
        .load()
        .unwrap()
        .jobs
        .into_iter()
        .find(|job| job.id == job_id)
        .unwrap();
    assert_eq!(
        read_ocr_sample_report(&after, &child_id, &sample_dir).unwrap(),
        good_report,
        "a failed retry must not invalidate the comparison already on screen"
    );

    // No manifest is left behind for either the successful or the failed run.
    let leftovers: Vec<_> = fs::read_dir(&sample_dir)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("manifest-"))
        .collect();
    assert!(leftovers.is_empty(), "stale manifests: {leftovers:?}");

    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_successful_resample_does_not_accumulate_report_files() {
    let root = temp_root("ocr-sample-no-accumulate");
    let store = MemoryStateStore::new(&root);
    let (job_id, child_id, _) = ocr_sample_ready_job(&root, &store);
    let sample_dir = ocr_sample_dir(&store, &job_id, &child_id);

    for _ in 0..3 {
        run_ocr_sample_with_executor(
            &store,
            &job_id,
            Some(&child_id),
            OCR_SAMPLE_PAGE_COUNT,
            &OcrSampleFixtureExecutor::default(),
        )
        .unwrap();
    }

    let reports: Vec<_> = fs::read_dir(&sample_dir)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("report-"))
        .collect();
    assert_eq!(
        reports.len(),
        1,
        "superseded reports were kept: {reports:?}"
    );

    let _ = fs::remove_dir_all(root);
}

/// The report path comes out of the state file, so a tampered artifact record
/// must not be able to make the reader hand the UI any file it can read.
#[test]
fn ocr_sample_read_refuses_a_report_outside_the_sample_directory() {
    let root = temp_root("ocr-sample-escape");
    let store = MemoryStateStore::new(&root);
    let (job_id, child_id, _) = ocr_sample_ready_job(&root, &store);
    let sample_dir = ocr_sample_dir(&store, &job_id, &child_id);
    let job = run_ocr_sample_with_executor(
        &store,
        &job_id,
        Some(&child_id),
        OCR_SAMPLE_PAGE_COUNT,
        &OcrSampleFixtureExecutor::default(),
    )
    .unwrap();

    let elsewhere = root.join("elsewhere.json");
    let smuggled = fs::read_to_string(
        job.children[0]
            .artifacts
            .iter()
            .find(|artifact| artifact.kind == "ocr_sample_report")
            .unwrap()
            .path
            .clone(),
    )
    .unwrap();
    fs::write(&elsewhere, &smuggled).unwrap();

    let mut tampered = job.clone();
    let artifact = tampered.children[0]
        .artifacts
        .iter_mut()
        .find(|artifact| artifact.kind == "ocr_sample_report")
        .unwrap();
    artifact.path = display_path(&elsewhere);
    // Same bytes, so the digest still matches: only the containment check can
    // catch this one.
    let err = read_ocr_sample_report(&tampered, &child_id, &sample_dir).unwrap_err();
    assert!(err.contains("outside the job's sample directory"), "{err}");

    let _ = fs::remove_dir_all(root);
}

/// The shape zotero_source_ref actually produces. Before this, the fingerprint
/// rode along as part of the file extension, so `is_pdf_path` was false for
/// every real Zotero attachment and the feature refused its main input.
#[test]
fn ocr_sample_resolves_a_zotero_source_ref_with_its_fingerprint() {
    let root = temp_root("ocr-sample-zotero-ref");
    let store = MemoryStateStore::new(&root);
    let (job_id, child_id, source_pdf) = ocr_sample_ready_job(&root, &store);

    let mut state = store.load().unwrap();
    let stored = state.jobs.iter_mut().find(|it| it.id == job_id).unwrap();
    stored.children[0].source.path = Some(zotero_source_ref(
        "ABCD1234",
        Some("d41d8cd98f00b204e9800998ecf8427e"),
        Some(&display_path(&source_pdf)),
    ));
    store.save(&state).unwrap();

    let executor = OcrSampleFixtureExecutor::default();
    run_ocr_sample_with_executor(
        &store,
        &job_id,
        Some(&child_id),
        OCR_SAMPLE_PAGE_COUNT,
        &executor,
    )
    .unwrap();

    // The manifest must name the real file, fingerprint stripped.
    assert_eq!(
        executor.manifests()[0]["sourcePdfPath"],
        serde_json::json!(display_path(&source_pdf))
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ocr_sample_names_an_unsynced_zotero_attachment_as_such() {
    let root = temp_root("ocr-sample-unsynced");
    let store = MemoryStateStore::new(&root);
    let (job_id, child_id, _) = ocr_sample_ready_job(&root, &store);

    let mut state = store.load().unwrap();
    let stored = state.jobs.iter_mut().find(|it| it.id == job_id).unwrap();
    // What zotero_source_ref returns when Zotero reported no output path.
    stored.children[0].source.path = Some(zotero_source_ref("ABCD1234", Some("d41d8c"), None));
    stored.children[0].route.clear();
    store.save(&state).unwrap();

    let err = run_ocr_sample_with_executor(
        &store,
        &job_id,
        Some(&child_id),
        OCR_SAMPLE_PAGE_COUNT,
        &OcrSampleFixtureExecutor::default(),
    )
    .unwrap_err();
    assert!(err.contains("not stored locally"), "{err}");

    let _ = fs::remove_dir_all(root);
}

#[test]
fn ocr_sample_local_path_strips_only_the_fingerprint_suffix() {
    assert_eq!(
        ocr_sample_local_path("/books/a.pdf#source_md5=abc"),
        Some(PathBuf::from("/books/a.pdf"))
    );
    // `#` is legal in a filename, so only the exact producer suffix comes off.
    assert_eq!(
        ocr_sample_local_path("/books/draft#2.pdf"),
        Some(PathBuf::from("/books/draft#2.pdf"))
    );
    assert_eq!(
        ocr_sample_local_path("/books/draft#2.pdf#source_md5=abc"),
        Some(PathBuf::from("/books/draft#2.pdf"))
    );
    assert_eq!(ocr_sample_local_path("zotero://attachment/KEY"), None);
    assert_eq!(
        ocr_sample_local_path("zotero://attachment/KEY#source_md5=abc"),
        None
    );
    assert_eq!(ocr_sample_local_path(""), None);
}

/// The worker exits 0 but the report is unusable. These are the likeliest real
/// failures -- a manifest whose reportPath the Python side resolved elsewhere,
/// or an engine response that did not survive validation -- and each has to
/// surface as its own message rather than as a panic or a silent success.
#[test]
fn ocr_sample_reports_a_worker_that_produced_no_usable_report() {
    /// What the stand-in worker does with the report path the manifest names.
    type ReportWriter = Box<dyn Fn(&Path) + Send + Sync>;

    struct WritingExecutor(ReportWriter);
    impl RunnerCommandExecutor for WritingExecutor {
        fn execute(&self, command: &RunnerCommand) -> Result<RunnerCommandResult, String> {
            let manifest: serde_json::Value =
                serde_json::from_str(&fs::read_to_string(&command.args[6]).unwrap()).unwrap();
            let report_path = PathBuf::from(manifest["projectRoot"].as_str().unwrap())
                .join(manifest["reportPath"].as_str().unwrap());
            (self.0)(&report_path);
            Ok(RunnerCommandResult::default())
        }
    }

    let cases: Vec<(&str, ReportWriter)> = vec![
        ("was not written", Box::new(|_: &Path| {})),
        (
            "invalid report JSON",
            Box::new(|path: &Path| fs::write(path, "not json").unwrap()),
        ),
        (
            "unsupported schema",
            Box::new(|path: &Path| {
                let report = serde_json::json!({
                    "schema": "ocr-sample-compare-report-v0",
                            "totalPages": 40,
                    "sampledPages": [11, 21, 30],
                    "characterBudget": OCR_SAMPLE_CHARACTER_BUDGET,
                    "engines": [],
                });
                fs::write(path, serde_json::to_string(&report).unwrap()).unwrap();
            }),
        ),
    ];

    for (expected, writer) in cases {
        let root = temp_root(&format!("ocr-sample-bad-{}", expected.replace(' ', "-")));
        let store = MemoryStateStore::new(&root);
        let (job_id, child_id, _) = ocr_sample_ready_job(&root, &store);
        let err = run_ocr_sample_with_executor(
            &store,
            &job_id,
            Some(&child_id),
            OCR_SAMPLE_PAGE_COUNT,
            &WritingExecutor(writer),
        )
        .unwrap_err();
        assert!(err.contains(expected), "expected {expected:?}, got {err}");
        // Nothing half-registered: a bad report must not become an artifact.
        let after = store
            .load()
            .unwrap()
            .jobs
            .into_iter()
            .find(|job| job.id == job_id)
            .unwrap();
        assert!(!after.children[0]
            .artifacts
            .iter()
            .any(|artifact| artifact.kind == "ocr_sample_report"));
        // Nor left on disk: names are per-run, so an unregisterable report
        // would otherwise accumulate one file per failed attempt.
        let sample_dir = ocr_sample_dir(&store, &job_id, &child_id);
        let leftovers: Vec<_> = fs::read_dir(&sample_dir)
            .unwrap()
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.starts_with("report-") || name.starts_with("manifest-"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "{expected}: left behind {leftovers:?}"
        );
        let _ = fs::remove_dir_all(root);
    }
}

/// The conversion stage scans the whole job output root recursively, and the
/// engine comparison lands inside it. Left visible, MinerU's sampled `part.md`
/// registers as the book's `markdown` artifact -- and for a route whose
/// converter emits no Markdown at all it becomes the *only* one, so the
/// translation project is built from three sampled pages instead of the book,
/// with a valid digest and no error anywhere.
#[test]
fn a_conversion_scan_ignores_the_ocr_sample_directory() {
    let root = temp_root("ocr-sample-scan-isolation");
    let store = MemoryStateStore::new(&root);
    let (job_id, child_id, _) = ocr_sample_ready_job(&root, &store);
    run_ocr_sample_with_executor(
        &store,
        &job_id,
        Some(&child_id),
        OCR_SAMPLE_PAGE_COUNT,
        &OcrSampleFixtureExecutor::default(),
    )
    .unwrap();

    let job_output = store.job_output_dir(&job_id);
    // The real conversion output, so the scan has something legitimate to find.
    let converted = job_output.join("book.md");
    fs::write(&converted, "# The actual book\n").unwrap();
    // What MinerU leaves behind mid-sample, if the scratch tree ever survives.
    let strays = ocr_sample_dir(&store, &job_id, &child_id)
        .join("work")
        .join("mineru");
    fs::create_dir_all(&strays).unwrap();
    fs::write(strays.join("part.md"), "three sampled pages\n").unwrap();

    let scanned = scan_artifacts(&job_output).unwrap();
    let markdown: Vec<_> = scanned
        .iter()
        .filter(|artifact| artifact.kind == "markdown")
        .map(|artifact| artifact.path.clone())
        .collect();
    assert_eq!(
        markdown,
        vec![display_path(&converted)],
        "the sample tree leaked into the conversion's artifacts"
    );
    // The report itself is registered by the sample command, not by this scan.
    // Compared against the real sample directory rather than by substring: the
    // temp root's own name contains "ocr-sample" and would match either way.
    let sample_dir = ocr_sample_dir(&store, &job_id, &child_id);
    assert!(
        !scanned
            .iter()
            .any(|artifact| Path::new(&artifact.path).starts_with(&sample_dir)),
        "sample files were scanned as conversion output: {:?}",
        scanned
            .iter()
            .map(|artifact| artifact.path.as_str())
            .collect::<Vec<_>>()
    );

    let _ = fs::remove_dir_all(root);
}

// ---- Layout-preserving PDF track (BabelDOC) --------------------------------

fn layout_route(route_kind: &str, source_ref: &str) -> BookPipelineRouteItem {
    BookPipelineRouteItem {
        id: "ATTACH1".into(),
        title: "Fixture attachment".into(),
        source_kind: "zotero_attachment".into(),
        source_ref: source_ref.into(),
        route_kind: route_kind.into(),
        can_run: true,
        blocked_reason: None,
        summary: "fixture route".into(),
        route_override: None,
    }
}

fn layout_job(route: Vec<BookPipelineRouteItem>) -> BookPipelineJob {
    let mut job: BookPipelineJob = serde_json::from_value(serde_json::json!({
        "id": "job-layout",
        "mode": MODE_LAYOUT_PRESERVING,
        "source": { "kind": "zotero_attachment", "title": "Fixture attachment" },
        "route": [],
        "status": "routed",
        "currentStep": "Route preview recorded",
        "lastError": null,
        "logSummary": [],
        "artifacts": [],
        "outputDir": null,
        "attempts": 0,
        "createdAt": "2026-08-01T00:00:00Z",
        "updatedAt": "2026-08-01T00:00:00Z"
    }))
    .unwrap();
    job.route = route;
    job
}

fn layout_package_root(root: &Path) -> PathBuf {
    let manifest = root.join("packages").join("layout-pdf");
    fs::create_dir_all(&manifest).unwrap();
    fs::write(
        manifest.join("pyproject.toml"),
        "[project]\nname = \"layout-pdf\"\n",
    )
    .unwrap();
    root.to_path_buf()
}

#[test]
fn the_layout_track_stops_after_extract() {
    // Two stages is the whole point: no split, no approval gates, no EPUB build.
    assert_eq!(
        ordered_child_stage_ids(MODE_LAYOUT_PRESERVING, false),
        vec!["route", "extract"]
    );
    // A legacy translation flag must not graft the reflow stages back on; this
    // mode postdates every state file such a flag can describe.
    assert_eq!(
        ordered_child_stage_ids(MODE_LAYOUT_PRESERVING, true),
        vec!["route", "extract"]
    );
}

#[test]
fn the_layout_track_gets_no_item_index_stage() {
    // `ensure_item_index_stage` keys off the source kind, and a layout job's
    // source is a Zotero attachment like any other. Without the mode guard it
    // would append a third stage that has no Markdown to index and never runs.
    let mut child = BookPipelineChildJob {
        source: fake_direct_zotero_source(),
        stages: ordered_child_stage_ids(MODE_LAYOUT_PRESERVING, false)
            .into_iter()
            .map(|stage_id| BookPipelineStage {
                stage_id: stage_id.into(),
                status: STATUS_PENDING.into(),
                ..BookPipelineStage::default()
            })
            .collect(),
        ..BookPipelineChildJob::default()
    };

    ensure_item_index_stage(&mut child, MODE_LAYOUT_PRESERVING);

    assert_eq!(
        child
            .stages
            .iter()
            .map(|stage| stage.stage_id.as_str())
            .collect::<Vec<_>>(),
        vec!["route", "extract"]
    );
}

#[test]
fn the_reflow_track_still_gets_its_item_index_stage() {
    // The guard above must be scoped to the layout mode, not a blanket skip.
    let mut child = BookPipelineChildJob {
        source: fake_direct_zotero_source(),
        stages: vec![
            BookPipelineStage {
                stage_id: "route".into(),
                status: STATUS_COMPLETED.into(),
                ..BookPipelineStage::default()
            },
            BookPipelineStage {
                stage_id: "extract".into(),
                status: STATUS_PENDING.into(),
                ..BookPipelineStage::default()
            },
        ],
        ..BookPipelineChildJob::default()
    };

    ensure_item_index_stage(&mut child, MODE_CONVERT_THEN_TRANSLATE);

    assert!(child.stages.iter().any(|stage| stage.stage_id == "index"));
}

#[test]
fn the_layout_track_never_hands_off_to_translation() {
    assert!(!should_handoff_after_run(MODE_LAYOUT_PRESERVING));
}

#[test]
fn the_layout_command_runs_the_babeldoc_wrapper_from_the_workspace() {
    let root = temp_root("layout-command");
    let repo_root = layout_package_root(&root);
    let source_pdf = root.join("Weber 1922.pdf");
    fs::write(&source_pdf, b"%PDF-1.7\n").unwrap();
    let output = root.join("output");
    let job = layout_job(vec![layout_route(
        "direct_text",
        &display_path(&source_pdf),
    )]);

    let command = build_layout_pdf_command_for_root(&job, &output, &repo_root).unwrap();

    assert_eq!(command.label, LAYOUT_PDF_COMMAND_LABEL);
    assert_eq!(command.program, PathBuf::from("uv"));
    assert_eq!(
        command.args,
        vec![
            "run".to_string(),
            "--package".to_string(),
            "layout-pdf".to_string(),
            // Without the extra the subprocess starts and then dies on an
            // ImportError, because babeldoc is deliberately not in the shared venv.
            "--extra".to_string(),
            "babeldoc".to_string(),
            "layout-pdf".to_string(),
            "--input".to_string(),
            display_path(&source_pdf),
            "--output-dir".to_string(),
            display_path(&output),
        ]
    );
    assert_eq!(command.cwd, Some(repo_root));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn the_layout_command_strips_the_worker_fingerprint_from_the_source_path() {
    // Every real Zotero route carries `#source_md5=...` on its source_ref. Taken
    // literally that is a path that does not exist and does not end in .pdf, so
    // this fails for every real book while hand-built fixtures pass.
    let root = temp_root("layout-fingerprint");
    let repo_root = layout_package_root(&root);
    let source_pdf = root.join("Weber 1922.pdf");
    fs::write(&source_pdf, b"%PDF-1.7\n").unwrap();
    let source_ref = format!(
        "{}#source_md5=0123456789abcdef0123456789abcdef",
        display_path(&source_pdf)
    );
    let job = layout_job(vec![layout_route("direct_text", &source_ref)]);

    let command =
        build_layout_pdf_command_for_root(&job, &root.join("output"), &repo_root).unwrap();

    let input = command
        .args
        .windows(2)
        .find(|pair| pair[0] == "--input")
        .map(|pair| pair[1].clone())
        .unwrap();
    assert_eq!(input, display_path(&source_pdf));
    assert!(!input.contains("source_md5"));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_hash_in_the_filename_survives_fingerprint_stripping() {
    // `#` is legal in a filename. Splitting on a bare `#` -- or on the first
    // occurrence of the marker -- would truncate `draft#2.pdf` to `draft` and
    // report the book as missing.
    let root = temp_root("layout-hash-name");
    let repo_root = layout_package_root(&root);
    let source_pdf = root.join("draft#2.pdf");
    fs::write(&source_pdf, b"%PDF-1.7\n").unwrap();
    let source_ref = format!(
        "{}#source_md5=0123456789abcdef0123456789abcdef",
        display_path(&source_pdf)
    );
    let job = layout_job(vec![layout_route("direct_text", &source_ref)]);

    let command =
        build_layout_pdf_command_for_root(&job, &root.join("output"), &repo_root).unwrap();

    assert!(command
        .args
        .windows(2)
        .any(|pair| pair[0] == "--input" && pair[1] == display_path(&source_pdf)));

    let _ = fs::remove_dir_all(root);
}

#[test]
fn the_layout_track_refuses_a_scanned_book() {
    let root = temp_root("layout-scanned");
    let repo_root = layout_package_root(&root);
    let source_pdf = root.join("scan.pdf");
    fs::write(&source_pdf, b"%PDF-1.7\n").unwrap();
    let job = layout_job(vec![layout_route(
        "remote_paddleocr",
        &display_path(&source_pdf),
    )]);

    let error =
        build_layout_pdf_command_for_root(&job, &root.join("output"), &repo_root).unwrap_err();

    assert!(
        error.contains("only available for text PDFs"),
        "unexpected error: {error}"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn the_layout_track_refuses_more_than_one_book_at_a_time() {
    let root = temp_root("layout-multi");
    let repo_root = layout_package_root(&root);
    let first = root.join("first.pdf");
    let second = root.join("second.pdf");
    fs::write(&first, b"%PDF-1.7\n").unwrap();
    fs::write(&second, b"%PDF-1.7\n").unwrap();
    let job = layout_job(vec![
        layout_route("direct_text", &display_path(&first)),
        layout_route("direct_text", &display_path(&second)),
    ]);

    let error =
        build_layout_pdf_command_for_root(&job, &root.join("output"), &repo_root).unwrap_err();

    assert!(
        error.contains("one book at a time"),
        "unexpected error: {error}"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn the_layout_track_refuses_a_source_that_is_not_a_pdf() {
    let root = temp_root("layout-not-pdf");
    let repo_root = layout_package_root(&root);
    let source = root.join("book.epub");
    fs::write(&source, b"PK\x03\x04").unwrap();
    let job = layout_job(vec![layout_route("direct_text", &display_path(&source))]);

    let error =
        build_layout_pdf_command_for_root(&job, &root.join("output"), &repo_root).unwrap_err();

    assert!(
        error.contains("only accepts PDFs"),
        "unexpected error: {error}"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn the_layout_track_reports_a_missing_source_before_spawning_anything() {
    let root = temp_root("layout-missing");
    let repo_root = layout_package_root(&root);
    let job = layout_job(vec![layout_route(
        "direct_text",
        &display_path(&root.join("absent.pdf")),
    )]);

    let error =
        build_layout_pdf_command_for_root(&job, &root.join("output"), &repo_root).unwrap_err();

    assert!(
        error.contains("Source PDF not found"),
        "unexpected error: {error}"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn the_layout_mode_takes_precedence_over_the_source_kind() {
    // The dispatch in build_runner_command_with_root has to check the mode
    // first, or a Zotero attachment routes to the OCR worker as usual and the
    // track silently never runs.
    let root = temp_root("layout-dispatch");
    let source_pdf = root.join("book.pdf");
    fs::create_dir_all(&root).unwrap();
    fs::write(&source_pdf, b"%PDF-1.7\n").unwrap();
    let job = layout_job(vec![layout_route(
        "direct_text",
        &display_path(&source_pdf),
    )]);

    // The OCR root argument is what a reflow job would use; a layout job must
    // ignore it entirely. Only the label is asserted: resolving the layout
    // package needs the real repo root, which this test does not have.
    let dispatched = build_runner_command_with_root(&job, &root.join("output"), Some(&root));
    match dispatched {
        Ok(command) => assert_eq!(command.label, LAYOUT_PDF_COMMAND_LABEL),
        Err(error) => assert!(
            error.contains("Layout-preserving PDF package not found"),
            "dispatched to the OCR worker instead of the layout track: {error}"
        ),
    }

    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_pdf_is_registered_as_an_artifact() {
    let root = temp_root("layout-artifact");
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("Weber 1922.zh-CN.bilingual.pdf"), b"%PDF-1.7\n").unwrap();

    let artifacts = scan_artifacts(&root).unwrap();

    assert_eq!(artifacts.len(), 1);
    assert_eq!(artifacts[0].kind, "pdf");
    assert_eq!(artifact_default_stage("pdf"), "extract");

    let _ = fs::remove_dir_all(root);
}

#[test]
fn the_ocr_workers_page_chunks_are_not_registered_as_artifacts() {
    // zotero_llm_worker.py splits a long book into page ranges under
    // .state/chunks before uploading them. Those splits are PDFs, so teaching
    // artifact_kind about PDFs would otherwise register dozens of them per book
    // as deliverables -- while .state/staging, where the finished Markdown
    // lives, has to keep being scanned.
    let root = temp_root("layout-chunks");
    let chunks = root
        .join(".state")
        .join("chunks")
        .join("ATTACH1")
        .join("md5");
    let staging = root.join(".state").join("staging").join("ATTACH1");
    fs::create_dir_all(&chunks).unwrap();
    fs::create_dir_all(&staging).unwrap();
    fs::write(chunks.join("pages-0001-0050.pdf"), b"%PDF-1.7\n").unwrap();
    fs::write(chunks.join("pages-0051-0100.pdf"), b"%PDF-1.7\n").unwrap();
    fs::write(staging.join("book.md"), "# Book\n").unwrap();

    let artifacts = scan_artifacts(&root).unwrap();

    assert_eq!(
        artifacts
            .iter()
            .map(|artifact| artifact.kind.as_str())
            .collect::<Vec<_>>(),
        vec!["markdown"],
        "registered a working file as a deliverable: {artifacts:?}"
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_directory_named_chunks_outside_the_worker_state_tree_is_still_scanned() {
    // The skip is anchored on .state/chunks, not on the name alone: a book whose
    // own folder happens to be called "chunks" must not vanish.
    let root = temp_root("layout-chunks-elsewhere");
    let chunks = root.join("chunks");
    fs::create_dir_all(&chunks).unwrap();
    fs::write(chunks.join("book.zh-CN.bilingual.pdf"), b"%PDF-1.7\n").unwrap();

    let artifacts = scan_artifacts(&root).unwrap();

    assert_eq!(artifacts.len(), 1);
    assert_eq!(artifacts[0].kind, "pdf");

    let _ = fs::remove_dir_all(root);
}

#[test]
fn babeldoc_warning_counts_reach_the_job_log() {
    let root = temp_root("layout-warnings");
    fs::create_dir_all(&root).unwrap();
    let stdout = "BOOK_PIPELINE_MARKER warning=large_page count=3\n\
                  BOOK_PIPELINE_MARKER warning=other count=1\n";

    let markers = parse_allowlisted_worker_markers(stdout, &[root.as_path()]);

    assert_eq!(
        markers,
        vec![
            "worker marker: warning=large_page count=3".to_string(),
            "worker marker: warning=other count=1".to_string(),
        ]
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn an_unknown_warning_kind_is_dropped_rather_than_carried_through() {
    // The marker parser is the boundary that keeps free text -- BabelDOC warns
    // with interpolated page numbers and sometimes file paths -- out of a job
    // log. A kind not in LAYOUT_PDF_WARNING_KINDS must not become one.
    let root = temp_root("layout-warning-unknown");
    fs::create_dir_all(&root).unwrap();
    let stdout = "BOOK_PIPELINE_MARKER warning=/library/storage/ABCD1234/secret.pdf count=1\n";

    let markers = parse_allowlisted_worker_markers(stdout, &[root.as_path()]);

    assert_eq!(markers, vec!["worker marker: count=1".to_string()]);

    let _ = fs::remove_dir_all(root);
}

#[test]
fn the_layout_track_gets_a_translation_sized_timeout() {
    // A 400-page monograph is one LLM call per paragraph plus a first-run model
    // download; the default two hours would stop a healthy run mid-book.
    let command = RunnerCommand {
        kind: RunnerCommandKind::Process,
        label: LAYOUT_PDF_COMMAND_LABEL.into(),
        program: PathBuf::from("uv"),
        args: Vec::new(),
        env: Vec::new(),
        cwd: None,
        output_dir: PathBuf::from("/tmp"),
        attempts: 0,
        accepted_exit_codes: vec![0],
    };

    assert_eq!(
        runner_command_timeout(&command),
        Duration::from_secs(12 * 60 * 60)
    );
}

/// Stands in for the BabelDOC subprocess: writes the one file the real wrapper
/// writes, so the lifecycle can be exercised without a provider or a Keychain.
struct LayoutPdfFixtureRunner;

impl PipelineRunner for LayoutPdfFixtureRunner {
    fn run(&self, _job: &BookPipelineJob, output_dir: &Path) -> Result<RunnerOutput, String> {
        fs::create_dir_all(output_dir).unwrap();
        fs::write(
            output_dir.join("Weber 1922.zh-CN.bilingual.pdf"),
            b"%PDF-1.7 bilingual\n",
        )
        .unwrap();
        Ok(RunnerOutput {
            log_summary: vec![format!("{LAYOUT_PDF_COMMAND_LABEL} completed")],
            artifacts: scan_artifacts(output_dir)?,
            collection_items: Vec::new(),
            output_dir: Some(output_dir.to_path_buf()),
            current_step: None,
        })
    }
}

#[test]
fn a_layout_job_completes_at_extract_with_the_bilingual_pdf_registered() {
    let root = temp_root("layout-lifecycle");
    fs::create_dir_all(&root).unwrap();
    let store = BookPipelineStore::for_test(&root);
    let job = queue_job(
        &store,
        fake_direct_zotero_source(),
        MODE_LAYOUT_PRESERVING.into(),
        BookPipelinePreviewConfig {
            has_paddleocr_credentials: true,
            ..BookPipelinePreviewConfig::default()
        },
    )
    .unwrap();

    // Queued shape: two stages, and no translation handoff row on the route.
    let child = &job.children[0];
    assert_eq!(
        child
            .stages
            .iter()
            .map(|stage| stage.stage_id.as_str())
            .collect::<Vec<_>>(),
        vec!["route", "extract"]
    );
    assert!(
        !job.route
            .iter()
            .any(|item| item.route_kind == "translation_handoff"),
        "the layout track must not queue a translation handoff: {:?}",
        job.route
    );

    let completed = run_job(&store, &LayoutPdfFixtureRunner, &job.id).unwrap();

    assert_eq!(completed.status, STATUS_COMPLETED);
    assert_eq!(
        completed
            .artifacts
            .iter()
            .map(|artifact| artifact.kind.as_str())
            .collect::<Vec<_>>(),
        vec!["pdf"]
    );
    // No gate is ever raised: the whole point of this track is one pass.
    assert!(
        completed
            .children
            .iter()
            .flat_map(|child| child.stages.iter())
            .all(|stage| stage.status != STATUS_WAITING_FOR_APPROVAL),
        "the layout track must not raise an approval gate: {:?}",
        completed.children
    );
    assert!(
        completed.approval_references.is_empty(),
        "unexpected approval references: {:?}",
        completed.approval_references
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn a_layout_job_never_takes_the_reflow_source_shortcuts() {
    // `zotero_filter` -- the island's Zotero title search -- is a batch source,
    // and CommandPipelineRunner::run short circuits batch sources to the OCR
    // worker before it ever builds a command. Without the mode check in that
    // predicate, a layout job queued from a title search runs an OCR conversion
    // and the whole track is silently skipped.
    let mut job = layout_job(vec![layout_route("direct_text", "/books/book.pdf")]);

    for source_kind in ["zotero_filter", "zotero_collection", "markdown_source"] {
        job.source.kind = source_kind.into();

        job.mode = MODE_LAYOUT_PRESERVING.into();
        assert!(
            !takes_reflow_source_shortcut(&job),
            "a layout job with a {source_kind} source was shortcut past the layout track"
        );

        // The same predicate must keep saying yes for the reflow track, or this
        // guard has quietly disabled the shortcuts for everyone.
        job.mode = MODE_CONVERT_THEN_TRANSLATE.into();
        assert!(
            takes_reflow_source_shortcut(&job),
            "the reflow track lost its {source_kind} shortcut"
        );
    }
}

#[test]
fn a_finished_layout_job_opens_the_bilingual_pdf_itself() {
    // The layout track builds no reading project, so without its own navigation
    // target a finished book falls back to "Open workspace" and leaves the user
    // to pick the PDF out of the job output directory themselves.
    let root = temp_root("layout-open-target");
    fs::create_dir_all(&root).unwrap();
    let store = BookPipelineStore::for_test(&root);
    let job = queue_job(
        &store,
        fake_direct_zotero_source(),
        MODE_LAYOUT_PRESERVING.into(),
        BookPipelinePreviewConfig {
            has_paddleocr_credentials: true,
            ..BookPipelinePreviewConfig::default()
        },
    )
    .unwrap();

    let completed = run_job(&store, &LayoutPdfFixtureRunner, &job.id).unwrap();

    assert_eq!(completed.status, STATUS_COMPLETED);
    let open_target = completed
        .open_target
        .as_ref()
        .expect("a finished layout job must offer something to open");
    assert_eq!(open_target.kind, "bilingual_pdf");
    assert_eq!(open_target.action_label, "Open bilingual PDF");
    let target = completed
        .navigation_targets
        .iter()
        .find(|target| target.target_id == open_target.target_id)
        .expect("the selected target must be registered");
    assert!(
        target.path.ends_with(".pdf"),
        "expected the bilingual PDF, got {}",
        target.path
    );
}

#[test]
fn a_reflow_job_keeps_its_own_open_target() {
    // The arm added for the layout track sits ahead of the reading-output one,
    // so it has to be scoped by mode or every finished book claims to be a PDF.
    let root = temp_root("layout-open-target-reflow");
    fs::create_dir_all(&root).unwrap();
    let store = BookPipelineStore::for_test(&root);
    let job = queue_job(
        &store,
        fake_direct_zotero_source(),
        MODE_CONVERT_THEN_TRANSLATE.into(),
        BookPipelinePreviewConfig {
            has_paddleocr_credentials: true,
            ..BookPipelinePreviewConfig::default()
        },
    )
    .unwrap();

    // Same runner, so the same PDF lands in the job output directory. Only the
    // mode differs, and that alone must keep the target off this job.
    let ran = run_job(&store, &LayoutPdfFixtureRunner, &job.id).unwrap();

    assert!(
        !ran.navigation_targets
            .iter()
            .any(|target| target.kind == "bilingual_pdf"),
        "the reflow track must not claim a bilingual PDF target: {:?}",
        ran.navigation_targets
    );

    let _ = fs::remove_dir_all(root);
}

#[test]
fn the_layout_track_is_enqueueable() {
    // #96 closed the queue boundary to unknown modes. A new track that is not on
    // that list is refused before it ever builds a command, so the whole feature
    // is dead on arrival -- and the only symptom is a queue-time error message.
    assert!(validate_enqueue_mode(MODE_LAYOUT_PRESERVING).is_ok());
    assert!(ENQUEUEABLE_MODES.contains(&MODE_LAYOUT_PRESERVING));
}
